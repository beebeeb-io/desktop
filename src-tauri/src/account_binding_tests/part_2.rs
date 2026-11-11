//! Split out of `account_binding_tests.rs` for the file-size ratchet; the tests are unchanged.

use super::*;

#[test]
fn a_start_never_overwrites_an_engine_in_the_slot() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let fx = AuthorizeFixture::with_session(&alice());
        let task = tokio::spawn(async {});
        *fx.acct.engine.lock().await = Some(crate::runner::EngineRunner::for_test_with_task(task));
        let dir = tempfile::tempdir().unwrap();
        let slot = fx.acct.engine.lock().await;
        let error = authorize_engine_start(
            &fx.state,
            &fx.acct,
            &slot,
            &LocalDataPaths::for_test(dir.path(), dir.path()),
        )
        .expect_err("an occupied slot is never started over");
        assert_eq!(error, ENGINE_SLOT_OCCUPIED_ERROR);
        assert!(!dir.path().join("state.db").exists(), "and nothing was bound");
    });
}

#[test]
fn a_start_refuses_while_an_earlier_stop_is_unconfirmed_before_it_binds() {
    let fx = AuthorizeFixture::with_session(&alice());
    fx.acct.engine_stop_unconfirmed.store(true, Ordering::SeqCst);
    let dir = tempfile::tempdir().unwrap();
    let slot = fx.acct.engine.try_lock().unwrap();
    let error = authorize_engine_start(
        &fx.state,
        &fx.acct,
        &slot,
        &LocalDataPaths::for_test(dir.path(), dir.path()),
    )
    .expect_err("refused");
    assert_eq!(error, ENGINE_START_REFUSED_ERROR);
    assert!(!dir.path().join("state.db").exists(), "a refused start binds nothing");
}

pub(super) fn profile_of(user_id: &str, email: &str) -> account_dto::AccountProfile {
    serde_json::from_str::<account_dto::AccountProfile>(&format!(
        r#"{{"user_id":"{user_id}","email":"{email}","email_verified":true,"created_at":"2026-01-01T00:00:00Z"}}"#
    ))
    .unwrap()
}

#[test]
fn the_session_identity_takes_the_profile_only_when_it_is_this_sessions() {
    let profile = |email: &str| profile_of("u-a", email);
    assert_eq!(
        identity_of_session(Some("a@beebeeb.io"), Some(&profile("a@beebeeb.io"))),
        Identity::new(Some("u-a"), Some("a@beebeeb.io"))
    );
    assert_eq!(
        identity_of_session(Some("a@beebeeb.io"), Some(&profile("A@beebeeb.io"))),
        Identity::new(None, Some("a@beebeeb.io")),
        "a profile whose email differs only in case is another account's: never folded"
    );
    assert_eq!(
        identity_of_session(Some("a@beebeeb.io"), Some(&profile("someone-else@beebeeb.io"))),
        Identity::new(None, Some("a@beebeeb.io")),
        "a profile of another email is ignored"
    );
    assert_eq!(
        identity_of_session(Some("a@beebeeb.io"), None),
        Identity::new(None, Some("a@beebeeb.io")),
        "offline: the email alone"
    );
    assert_eq!(
        identity_of_session(None, Some(&profile("a@beebeeb.io"))),
        Identity::default(),
        "no session email: a cached profile is nobody's to borrow"
    );
    assert_eq!(
        identity_of_session(Some("  "), Some(&profile("a@beebeeb.io"))),
        Identity::default(),
        "an empty email is no email"
    );
}

/// F1: the identity of a session is its own. A session with no email is unidentified even when a cached profile
/// of some other account exists, and an unidentified session against a recorded owner is refused with a reason
/// and deletes nothing.
#[test]
fn an_unidentified_session_is_refused_and_takes_no_cached_profile_for_its_own() {
    let local = local_data_of(Some(alice()));
    let fx = AuthorizeFixture::without_session();
    *fx.acct.session.lock().unwrap() = Some(Session {
        token: "tok-b".into(),
        master_key: [5u8; 32],
        email: None,
    });
    *fx.acct.auth_email.lock().unwrap() = None;
    *fx.acct.cached_profile.lock().unwrap() = Some(profile_of("u-a", "a@beebeeb.io"));
    let (_, identity) = keys_for_engine_start(&fx.acct).expect("keys");
    assert_eq!(identity, Identity::default(), "the profile is not borrowed");
    let slot = fx.acct.engine.try_lock().unwrap();
    let result = authorize_engine_start(
        &fx.state,
        &fx.acct,
        &slot,
        &LocalDataPaths::for_test(local.dir.path(), &local.staging),
    );
    assert_eq!(
        result.err(),
        Some(crate::account_binding::IDENTITY_UNKNOWN.to_string()),
        "an unidentified session against a recorded owner"
    );
    assert_eq!(local.data.db.owner().unwrap(), Some(alice()));
    assert_eq!(
        local.data.db.list_due_operations(i64::MAX).unwrap().len(),
        1,
        "nothing is deleted either"
    );
}

/// F1: the keys and the identity a start checks come from the same session, read together. The mirrored
/// account fields (`auth_email`, `cached_profile`) are never where the identity comes from, so they cannot
/// disagree with the keys.
#[test]
fn a_start_pairs_the_keys_with_the_identity_of_the_same_session() {
    let local = local_data_of(Some(alice()));
    let fx = AuthorizeFixture::with_session(&bob());
    // The mirrored fields describe another account; they play no part.
    *fx.acct.auth_email.lock().unwrap() = Some("a@beebeeb.io".into());
    *fx.acct.cached_profile.lock().unwrap() = Some(profile_of("u-a", "a@beebeeb.io"));
    let (_, identity) = keys_for_engine_start(&fx.acct).expect("keys");
    assert_eq!(
        identity,
        Identity::new(None, Some("b@beebeeb.io")),
        "the identity is the session's own"
    );
    let slot = fx.acct.engine.try_lock().unwrap();
    let result = authorize_engine_start(
        &fx.state,
        &fx.acct,
        &slot,
        &LocalDataPaths::for_test(local.dir.path(), &local.staging),
    );
    if RESET_ALLOWED {
        // On a Mac b's engine then waits for the previous account's Finder domain to be removed (F3).
        if cfg!(target_os = "macos") {
            assert!(
                matches!(result, Ok(StartPermit::FinderRemovalOwed)),
                "b waits for the removal: {result:?}"
            );
        } else {
            assert!(
                matches!(result, Ok(StartPermit::Go(_))),
                "b starts, on a clean slate: {result:?}"
            );
        }
        assert!(
            local.data.db.list_due_operations(i64::MAX).unwrap().is_empty(),
            "a's queue is not b's"
        );
        assert_eq!(
            local.data.db.owner().unwrap(),
            Some(Identity::new(None, Some("b@beebeeb.io")))
        );
    } else {
        assert_eq!(
            result.err(),
            Some(crate::account_binding::OTHER_ACCOUNT_ON_WINDOWS.to_string())
        );
        assert_eq!(
            local.data.db.list_due_operations(i64::MAX).unwrap().len(),
            1,
            "nothing discarded"
        );
    }
}

// ---- the log directory survives the reset (lead ruling 5) ----

/// The R10 reset removes the previous account's files through the sign-out purge's one deletion gate, and
/// that gate never reaches the lifecycle log directory, in the home or in the app's container.
#[test]
fn an_account_reset_never_deletes_the_lifecycle_log() {
    if !RESET_ALLOWED {
        return; // Windows refuses instead of resetting; nothing to delete.
    }
    let root = std::env::temp_dir().join(format!("bb-test-r10-logs-{}", uuid::Uuid::new_v4()));
    let logs = root.join("Library").join("Logs").join("Beebeeb");
    let container_logs = root
        .join("Library")
        .join("Containers")
        .join("io.beebeeb.app")
        .join("Data")
        .join("Library")
        .join("Logs")
        .join("Beebeeb");
    std::fs::create_dir_all(&logs).unwrap();
    std::fs::create_dir_all(&container_logs).unwrap();
    let ordinary = root.join("ordinary-cache.bin");
    let (log, rotated) = (logs.join("lifecycle.log"), container_logs.join("lifecycle.log.1"));
    for path in [&ordinary, &log, &rotated] {
        std::fs::write(path, b"x").unwrap();
    }
    let dir = tempfile::tempdir().unwrap();
    let db = crate::state_db::StateDb::open(dir.path().join("state.db")).unwrap();
    for (n, path) in [&ordinary, &log, &rotated].into_iter().enumerate() {
        db.track_staged_payload(path.to_str().unwrap(), None, false).unwrap();
        db.enqueue_operation(&crate::state_db::PendingOperation {
            op_id: format!("op-{n}"),
            kind: crate::state_db::OperationKind::UploadVersion,
            file_id: None,
            parent_id: None,
            target_path: Some(format!("/{n}.txt")),
            metadata_json: None,
            payload_path: Some(path.to_str().unwrap().to_string()),
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
    db.set_owner(&alice()).unwrap();
    let data = StateDbLocalData {
        db,
        sync_root: None,
        staging_dirs: Vec::new(),
        kept: Default::default(),
    };
    assert_eq!(bind_data_to_session(&AppState::default(), &data, &bob()), Ok(()));
    let survived = (log.exists(), rotated.exists(), ordinary.exists());
    let cleared = !data.db.has_account_data().unwrap();
    let _ = std::fs::remove_dir_all(&root);
    assert_eq!(
        survived,
        (true, true, false),
        "the log directory survives; the ordinary file does not"
    );
    assert!(cleared, "and the account's rows are all gone");
}

/// On a Mac a reset asks the reconciler for a Repair, so Finder drops the old listing and gets Beebeeb back
/// for the new account; the same account asks for nothing. The reconciler's end is a stand-in that only
/// receives, and the call returns without waiting for an answer.
#[cfg(target_os = "macos")]
#[test]
fn a_reset_asks_the_reconciler_for_a_repair_and_a_kept_account_asks_for_nothing() {
    use finder_setup::driver::{Event, FinderSetupHandle, FinderSetupView};
    use finder_setup::launch_location::LaunchLocation;
    let (handle, mut rx) = FinderSetupHandle::for_test(FinderSetupView::initial(LaunchLocation::Applications));
    let state = AppState::default();
    let _ = state.finder_setup.set(handle);

    let kept = local_data_of(Some(alice()));
    assert_eq!(bind_data_to_session(&state, &kept.data, &alice()), Ok(()));
    assert!(rx.try_recv().is_err(), "the same account asks for no repair");

    let reset = local_data_of(Some(alice()));
    assert_eq!(bind_data_to_session(&state, &reset.data, &bob()), Ok(()));
    let event = rx.try_recv().expect("a reset asks the reconciler for something");
    assert!(
        matches!(event, Event::Trigger(finder_setup::core::Trigger::Repair)),
        "a Repair, and nothing else"
    );
    assert!(rx.try_recv().is_err(), "once");
}

/// F4: the account reset sweeps orphan staged files too (plaintext copies no row points at), through the
/// staging directories its `StateDbLocalData` was given.
#[test]
fn an_account_reset_also_sweeps_orphan_staged_files() {
    if !RESET_ALLOWED {
        return;
    }
    let mut local = local_data_of(Some(alice()));
    let staging = std::env::temp_dir()
        .join(format!("bb-test-1834-f4-reset-{}", uuid::Uuid::new_v4()))
        .join("finder-writes");
    std::fs::create_dir_all(&staging).unwrap();
    let orphan = staging.join("orphan.bin");
    std::fs::write(&orphan, b"plaintext with no row").unwrap();
    local.data.staging_dirs = vec![staging.clone()];
    assert_eq!(bind_data_to_session(&AppState::default(), &local.data, &bob()), Ok(()));
    let swept = !orphan.exists();
    let _ = std::fs::remove_dir_all(staging.parent().unwrap());
    assert!(swept, "the orphan is gone");
}

/// F4 at the gate: an account reset that cannot remove a file starts nothing, leaves the owner and every row
/// where they were, and the retry (once the file can go) starts the engine on a clean slate.
#[test]
fn an_account_reset_that_cannot_remove_a_file_starts_nothing_until_it_can() {
    if !RESET_ALLOWED {
        return; // Windows refuses before any purge
    }
    let local = local_data_of(Some(alice()));
    // The journal's staged copy, not the queued upload's: on a Mac a waiting write's copy that is not a regular
    // file fails the reset's keep step, before the purge (task 1887, `the_r10_reset_keeps_before_it_purges`).
    let journal_copy = local.files[2].clone();
    std::fs::remove_file(&journal_copy).unwrap();
    std::fs::create_dir(&journal_copy).unwrap(); // a removal that fails
    let fx = AuthorizeFixture::with_session(&bob());
    let slot = fx.acct.engine.try_lock().unwrap();
    let paths = LocalDataPaths::for_test(local.dir.path(), &local.staging);
    let error = authorize_engine_start(&fx.state, &fx.acct, &slot, &paths).expect_err("nothing starts");
    assert!(error.contains("could not be removed"), "{error}");
    assert_eq!(
        local.data.db.owner().unwrap(),
        Some(alice()),
        "the new account is not recorded over data still there"
    );
    assert_eq!(
        local.data.db.list_due_operations(i64::MAX).unwrap().len(),
        1,
        "the queue is still there"
    );
    std::fs::remove_dir(&journal_copy).unwrap();
    let retry = authorize_engine_start(&fx.state, &fx.acct, &slot, &paths);
    // On a Mac the engine then waits for the previous account's Finder domain to be removed (F3).
    if cfg!(target_os = "macos") {
        assert!(
            matches!(retry, Ok(StartPermit::FinderRemovalOwed)),
            "the retry resets, then waits for the removal: {retry:?}"
        );
    } else {
        assert!(matches!(retry, Ok(StartPermit::Go(_))), "the retry starts: {retry:?}");
    }
    assert!(local.data.db.list_due_operations(i64::MAX).unwrap().is_empty());
    assert_eq!(local.data.db.owner().unwrap(), Some(bob()));
}

/// F2: a database the engine start itself creates is not the upgrade's, so the adoption window is shut from
/// the beginning: unowned data that shows up in it later is never adopted.
#[test]
fn a_database_the_engine_start_creates_never_opens_the_adoption_window() {
    let fx = AuthorizeFixture::with_session(&alice());
    let dir = tempfile::tempdir().unwrap();
    let slot = fx.acct.engine.try_lock().unwrap();
    assert!(matches!(
        authorize_after_removal(&fx, &slot, &LocalDataPaths::for_test(dir.path(), dir.path())),
        Ok(StartPermit::Go(_))
    ));
    let db = crate::state_db::StateDb::open(dir.path().join("state.db")).unwrap();
    db.clear_account_data(false).unwrap(); // no owner left ...
    rusqlite::Connection::open(dir.path().join("state.db"))
        .unwrap()
        .execute("INSERT INTO sync_state(key, value) VALUES ('cursor', '1')", []) // ... and unowned data
        .unwrap();
    assert!(
        !db.adopt_owner_once(Some(&bob())).unwrap(),
        "the window was shut when the database was created"
    );
    assert_eq!(db.owner().unwrap(), None);
}

/// F3: a reset on a Mac does not start the new account's engine in that call. It records a
/// Finder-removal debt in the reset's own transaction, asks the reconciler for a Repair (never waiting for it),
/// and every start refuses while the debt stands. A removal that FAILS releases nothing, so nothing starts and
/// nothing is reset a second time; a confirmed removal releases it, and then the engine starts.
#[cfg(target_os = "macos")]
#[test]
fn a_reset_whose_removal_fails_keeps_the_new_accounts_engine_from_starting_until_a_removal_is_confirmed() {
    use finder_setup::driver::{Event, FinderSetupHandle, FinderSetupView};
    use finder_setup::launch_location::LaunchLocation;
    fn start(
        fx: &AuthorizeFixture,
        slot: &mut tokio::sync::MutexGuard<'_, Option<crate::runner::EngineRunner>>,
        paths: &LocalDataPaths,
        started: &std::cell::Cell<u32>,
    ) -> Result<EngineStart, String> {
        start_engine_bound(&fx.state, &fx.acct, slot, paths, |_root, _token, _key| {
            started.set(started.get() + 1);
            crate::runner::EngineRunner::for_test_with_task(tokio::spawn(async {}))
        })
    }
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let (handle, mut rx) = FinderSetupHandle::for_test(FinderSetupView::initial(LaunchLocation::Applications));
        let local = local_data_of(Some(alice()));
        let fx = AuthorizeFixture::with_session(&bob());
        let _ = fx.state.finder_setup.set(handle);
        let mut slot = fx.acct.engine.lock().await;
        let paths = LocalDataPaths::for_test(local.dir.path(), &local.staging);
        let started = std::cell::Cell::new(0);

        // 1. b's start: the reset runs, the debt is recorded, Repair is asked for, and no engine starts.
        assert_eq!(
            start(&fx, &mut slot, &paths, &started),
            Ok(EngineStart::FinderRemovalOwed)
        );
        assert_eq!(started.get(), 0);
        assert!(slot.is_none());
        assert!(
            local.data.db.finder_removal_owed().unwrap(),
            "the debt is recorded with the reset"
        );
        assert!(
            local.data.db.list_due_operations(i64::MAX).unwrap().is_empty(),
            "the reset itself went through"
        );
        assert_eq!(local.data.db.owner().unwrap(), Some(bob()));
        assert!(
            matches!(rx.try_recv(), Ok(Event::Trigger(finder_setup::core::Trigger::Repair))),
            "a Repair was asked for"
        );

        // 2. The removal failed: nothing released the debt. Still no engine, and no second reset or Repair.
        assert_eq!(
            start(&fx, &mut slot, &paths, &started),
            Ok(EngineStart::FinderRemovalOwed)
        );
        assert_eq!(started.get(), 0);
        assert!(rx.try_recv().is_err(), "the refusal asks for nothing more");
        assert_eq!(local.data.db.owner().unwrap(), Some(bob()));
        // 2b. While the debt stands, even a DIFFERENT account that signs in is refused before anything is bound:
        //     no second reset, and the owner is not touched.
        let bobs_session = fx.acct.session.lock().unwrap().take();
        *fx.acct.session.lock().unwrap() = Some(Session {
            token: "tok-a".into(),
            master_key: [4u8; 32],
            email: alice().email,
        });
        assert_eq!(
            start(&fx, &mut slot, &paths, &started),
            Ok(EngineStart::FinderRemovalOwed)
        );
        assert_eq!(
            local.data.db.owner().unwrap(),
            Some(bob()),
            "nothing was bound while the removal is owed"
        );
        *fx.acct.session.lock().unwrap() = bobs_session;

        // 3. A removal is confirmed (the reconciler's port does exactly this): the engine may start.
        clear_finder_removal_owed_in(local.dir.path());
        assert!(!local.data.db.finder_removal_owed().unwrap());
        assert_eq!(start(&fx, &mut slot, &paths, &started), Ok(EngineStart::Started));
        assert_eq!(started.get(), 1);
        assert!(slot.is_some());
    });
}

/// F3: the debt is read by the reconciler's facts (`finder_removal_owed_in`) and cleared by its port
/// (`clear_finder_removal_owed_in`), through the same database the gate reads.
#[cfg(target_os = "macos")]
#[test]
fn the_reconcilers_view_of_the_removal_debt_is_the_gates() {
    let dir = tempfile::tempdir().unwrap();
    assert!(!finder_removal_owed_in(dir.path()), "no database, no debt");
    let db = crate::state_db::StateDb::open(dir.path().join("state.db")).unwrap();
    assert!(!finder_removal_owed_in(dir.path()));
    db.set_finder_removal_owed(true).unwrap();
    assert!(finder_removal_owed_in(dir.path()));
    assert!(!db.has_account_data().unwrap(), "the debt is not account data");
    clear_finder_removal_owed_in(dir.path());
    assert!(!finder_removal_owed_in(dir.path()));
}

/// `purge` with the app's `desktop.toml` in a directory of its own. On a Mac a sign-out purge that completes clears
/// the unfinished sign-out marker in the file `DesktopConfig::path()` names (spec 2026-10-10 W13): a test that calls
/// the purge itself, outside the sign-out tests' purge gate, must never clear it in the sandbox those tests share.
#[cfg(not(target_os = "windows"))]
pub(super) fn with_own_config<R>(purge: impl FnOnce() -> R) -> R {
    let config = tempfile::tempdir().unwrap();
    crate::config::CONFIG_DIR_OVERRIDE.sync_scope(config.path().to_path_buf(), purge)
}

/// The sign-out purge, behaviourally (macOS and Linux), through the real function `clear_session_impl` calls and
/// a state dir of its own: it clears every account row and the owner, records a removal debt only when the
/// sign-out's Finder removal was not confirmed, and a failure of any step stops it with the sign-out sentence.
#[cfg(not(target_os = "windows"))]
#[test]
fn the_sign_out_purge_clears_everything_and_records_the_debt_only_when_the_removal_was_not_confirmed() {
    for (owe, expected) in [(false, false), (true, true)] {
        let local = local_data_of(Some(alice()));
        rusqlite::Connection::open(local.dir.path().join("state.db"))
            .unwrap()
            .execute("INSERT INTO sync_state(key, value) VALUES ('cursor', '9')", [])
            .unwrap();
        assert_eq!(
            with_own_config(|| purge_local_data_for_sign_out(Ok(local.dir.path().to_path_buf()), None, owe, None)),
            Ok(())
        );
        assert!(!local.data.db.has_account_data().unwrap(), "no account row survives");
        assert_eq!(local.data.db.owner().unwrap(), None, "and nobody owns what is left");
        assert_eq!(
            local.data.db.finder_removal_owed().unwrap(),
            expected,
            "debt, owe = {owe}"
        );
        for path in &local.files {
            assert!(!path.exists(), "{} is gone", path.display());
        }
    }
}

#[cfg(not(target_os = "windows"))]
#[test]
fn the_sign_out_purge_stops_with_the_sign_out_sentence_when_any_step_fails() {
    // A file that cannot be removed.
    let local = local_data_of(Some(alice()));
    std::fs::remove_file(&local.files[0]).unwrap();
    std::fs::create_dir(&local.files[0]).unwrap();
    let error = purge_local_data_for_sign_out(Ok(local.dir.path().to_path_buf()), None, false, None).unwrap_err();
    assert!(
        error.starts_with(SIGN_OUT_PURGE_FAILED) && error.contains("could not be removed"),
        "{error}"
    );
    assert_eq!(local.data.db.owner().unwrap(), Some(alice()), "nothing was cleared");
    assert_eq!(local.data.db.list_due_operations(i64::MAX).unwrap().len(), 1);
    // A database that cannot be opened, and a state dir that cannot be resolved.
    let broken = tempfile::tempdir().unwrap();
    std::fs::write(
        broken.path().join("state.db"),
        b"this is not a database, and it is long enough to be read as one: 0123456789",
    )
    .unwrap();
    assert!(
        purge_local_data_for_sign_out(Ok(broken.path().to_path_buf()), None, false, None)
            .unwrap_err()
            .starts_with(SIGN_OUT_PURGE_FAILED)
    );
    assert!(
        purge_local_data_for_sign_out(Err("not initialised".into()), None, false, None)
            .unwrap_err()
            .starts_with(SIGN_OUT_PURGE_FAILED)
    );
    // No database yet is a fresh install: nothing to purge.
    let fresh = tempfile::tempdir().unwrap();
    assert_eq!(
        purge_local_data_for_sign_out(Ok(fresh.path().to_path_buf()), None, true, None),
        Ok(())
    );
}

/// F3, shape: the gate refuses while a removal is owed, before it binds anything and again after a reset; and the
/// reset records the debt in its own transaction (a Mac only).
#[test]
fn the_gate_refuses_while_a_finder_removal_is_owed_and_a_reset_records_the_debt() {
    let production = production_source();
    let bind = code_of(&production, "fn bind_local_data_to_session(");
    let first = bind.find("finder_removal_owed()").expect("it reads the debt");
    let bound = bind.find("bind_data_to_session(").expect("it binds");
    let second = bind.rfind("finder_removal_owed()").unwrap();
    assert!(
        first < bound && bound < second,
        "the debt is read before the binding and again after it:\n{bind}"
    );
    assert_eq!(
        bind.matches("return Ok(BoundData::FinderRemovalOwed)").count(),
        2,
        "{bind}"
    );
    let reset = body_between(&production, "    fn reset(&self)", "\n    }\n");
    assert!(
        reset.contains("clear_account_data(cfg!(target_os = \"macos\"))"),
        "the reset owes the removal on a Mac:\n{reset}"
    );
    let bound = body_between(&production, "fn start_engine_bound(", "\n}\n");
    assert!(
        bound.contains("StartPermit::FinderRemovalOwed => return Ok(EngineStart::FinderRemovalOwed)"),
        "{bound}"
    );
}

/// This is a public repository: the comments in the account-binding code, the reconciler's core and the state
/// database say what the code guarantees, and never announce that something is unresolved. (The needles
/// are generic and split so this test does not match itself.)
#[test]
fn no_public_comment_announces_something_still_open() {
    let needles = [
        concat!("is still", " open"),
        concat!("not", " closed"),
        concat!("un", "mitigated"),
        concat!("known", " weakness"),
        concat!("un", "fixed"),
    ];
    let strip_tests =
        |source: &str| source[..source.find("#[cfg(test)]\nmod tests {").unwrap_or(source.len())].to_ascii_lowercase();
    let sources = [
        ("lib.rs", production_source().to_ascii_lowercase()),
        (
            "finder_setup/core.rs",
            strip_tests(&include_str!("../finder_setup/core.rs").replace("\r\n", "\n")),
        ),
        (
            "account_binding.rs",
            strip_tests(&include_str!("../../../crates/beebeeb-engine/src/account_binding.rs").replace("\r\n", "\n")),
        ),
        (
            "state_db.rs",
            strip_tests(&include_str!("../../../crates/beebeeb-engine/src/state_db.rs").replace("\r\n", "\n")),
        ),
    ];
    for (name, source) in &sources {
        for needle in needles {
            assert!(!source.contains(needle), "{name} says {needle:?}");
        }
    }
}

/// R1 (fix round 2): the adoption window shuts on EVERY way out of the startup restore, not only on the paths
/// that reach the adoption call: a Windows restore that is refused, a restore with no account, a session
/// already installed, a Keychain with nothing in it. The restore is a thin wrapper that shuts the window after
/// the real body has returned (by whatever return), and the real body adopts on what it loaded. The restore
/// needs a real window system, so this is a source pin; the window's behaviour is the state_db and gate tests.
#[test]
fn every_return_of_the_startup_restore_shuts_the_adoption_window() {
    let production = production_source();
    let wrapper = code_of(&production, "async fn restore_session_on_startup(");
    let statements: Vec<&str> = wrapper
        .lines()
        .skip(1)
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    assert_eq!(
        statements,
        [
            "restore_session_inner(app).await;",
            "adopt_unbound_local_data_at_startup(None);"
        ],
        "the wrapper is exactly: run the restore, then shut the window, with nothing that can skip it:\n{wrapper}"
    );
    let body = code_of(&production, "async fn restore_session_inner(");
    assert!(
        body.contains("return;"),
        "the real restore has early returns (that is the point of the wrapper)"
    );
}

/// R1: an engine start shuts a window that is still open, on a database that already existed (the startup pass
/// could not run, say because the state dir was not resolved yet). Unowned data that appears later is then
/// never adopted.
#[test]
fn an_engine_start_shuts_an_open_adoption_window_on_an_existing_database() {
    let local = local_data_of(None); // a database from before the upgrade: unowned rows, window open
    let fx = AuthorizeFixture::with_session(&alice());
    let slot = fx.acct.engine.try_lock().unwrap();
    let _ = authorize_engine_start(
        &fx.state,
        &fx.acct,
        &slot,
        &LocalDataPaths::for_test(local.dir.path(), &local.staging),
    );
    local.data.db.clear_account_data(false).unwrap();
    rusqlite::Connection::open(local.dir.path().join("state.db"))
        .unwrap()
        .execute("INSERT INTO sync_state(key, value) VALUES ('cursor', '1')", [])
        .unwrap();
    assert!(
        !local.data.db.adopt_owner_once(Some(&bob())).unwrap(),
        "the engine start shut the window"
    );
    assert_eq!(local.data.db.owner().unwrap(), None);
}

/// A signed-in account the way a sign-in builds it: the session email is `session_email(profile)` (the server's
/// own record and nothing typed) and the profile is cached, exactly what `desktop_login`, `desktop_login_2fa`
/// and `apply_session` do. `None`: the sign-in's account-record fetch failed, so the session has no identity.
pub(super) fn signed_in_via(profile: Option<&account_dto::AccountProfile>) -> AuthorizeFixture {
    let fx = AuthorizeFixture::without_session();
    let email = session_email(profile);
    *fx.acct.session.lock().unwrap() = Some(Session {
        token: "tok-r10".into(),
        master_key: [7u8; 32],
        email: email.clone(),
    });
    *fx.acct.auth_email.lock().unwrap() = email;
    *fx.acct.cached_profile.lock().unwrap() = profile.cloned();
    fx
}

/// An HTTP server on localhost that answers every request with `status_line` and `body`; the base URL.
fn answering(status_line: &'static str, body: String) -> String {
    answering_after(status_line, body, || {})
}

/// [`answering`], running `before_reply` after the request arrived and before the answer is sent: a change to
/// the world that happens WHILE a fetch is in flight.
pub(super) fn answering_after(
    status_line: &'static str,
    body: String,
    before_reply: impl Fn() + Send + 'static,
) -> String {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            before_reply();
            let response = format!(
                "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    base_url
}

/// R2 (fix round 2; round 3): the email a session is known by is the server's own spelling of it, and nothing
/// else. Typed text is not a parameter any more, so a sign-in that has no account record has no email.
#[test]
fn the_session_email_is_the_servers_spelling_and_nothing_else() {
    let profile = profile_of("u-1", "sam@beebeeb.io");
    assert_eq!(session_email(Some(&profile)).as_deref(), Some("sam@beebeeb.io"));
    assert_eq!(
        session_email(Some(&profile_of("u-1", "  sam@beebeeb.io "))).as_deref(),
        Some("sam@beebeeb.io"),
        "trimmed"
    );
    assert_eq!(
        session_email(Some(&profile_of("u-1", "  "))),
        None,
        "an empty email names nobody"
    );
    assert_eq!(session_email(None), None, "no account record: no email");
}

/// R2: one account is ONE owner whichever way it signs in, because the identity is the server's record and
/// never the text a sign-in was handed. The account `sam@x` (user id u-1) owns the data; its sign-in starts
/// without purging its own unsynced queue. The server's other account `Sam@x` (a different user id) is another
/// owner (the ids decide, whatever the emails say).
#[test]
fn one_account_is_one_owner_whichever_way_it_signs_in_and_case_twins_stay_apart() {
    let owner = Identity::new(Some("u-1"), Some("sam@beebeeb.io"));
    let server = profile_of("u-1", "sam@beebeeb.io");
    let fx = signed_in_via(Some(&server));
    let local = local_data_of(Some(owner.clone()));
    let slot = fx.acct.engine.try_lock().unwrap();
    let result = authorize_engine_start(
        &fx.state,
        &fx.acct,
        &slot,
        &LocalDataPaths::for_test(local.dir.path(), &local.staging),
    );
    assert!(
        matches!(result, Ok(StartPermit::Go(_))),
        "the same account starts: {result:?}"
    );
    assert_eq!(
        local.data.db.list_due_operations(i64::MAX).unwrap().len(),
        1,
        "its own queue is never purged"
    );
    assert_eq!(local.data.db.owner().unwrap(), Some(owner.clone()), "still one owner");
    for path in &local.files {
        assert!(path.exists(), "{} is kept", path.display());
    }
    // The server's other account `Sam@x` (a different user id): another owner.
    let twin = signed_in_via(Some(&profile_of("u-2", "Sam@beebeeb.io")));
    let local = local_data_of(Some(owner));
    let slot = twin.acct.engine.try_lock().unwrap();
    let result = authorize_engine_start(
        &twin.state,
        &twin.acct,
        &slot,
        &LocalDataPaths::for_test(local.dir.path(), &local.staging),
    );
    if RESET_ALLOWED {
        assert!(
            !matches!(result, Ok(StartPermit::Go(_)) if !local.data.db.list_due_operations(i64::MAX).unwrap().is_empty()),
            "{result:?}"
        );
        assert!(
            local.data.db.list_due_operations(i64::MAX).unwrap().is_empty(),
            "the twin does not inherit the queue"
        );
    } else {
        assert_eq!(
            result.err(),
            Some(crate::account_binding::OTHER_ACCOUNT_ON_WINDOWS.to_string())
        );
    }
}

/// R2, wiring: every sign-in names the session by the server's profile and by nothing it was handed, and the
/// browser handoff fetches the profile and passes NO fallback email (round 3: a failed fetch leaves the session
/// unidentified instead of falling back to the web client's text).
#[test]
fn every_sign_in_names_the_session_by_the_servers_email() {
    let production = production_source();
    for signature in ["async fn desktop_login(", "async fn desktop_login_2fa("] {
        let code = code_of(&production, signature);
        let canonical = code
            .find("session_email(Some(&profile))")
            .unwrap_or_else(|| panic!("{signature}: the profile names the email"));
        // The token, the profile and the flags are written in the sign-in's turn, by `store_first_sign_in` (Task 12).
        let stored = code
            .find("store_first_sign_in(&state, &acct, &mut turn, &session_token, &email, profile)")
            .expect("the profile is cached by the checked store");
        assert!(
            canonical < stored,
            "{signature}: the email is read from the profile BEFORE it is moved into the cache, and the Keychain gets it"
        );
    }
    let store = code_of(&production, "fn store_first_sign_in(");
    assert!(
        store
            .find("persist_session_token_to_keychain(&write, acct.id.as_str(), token, Some(email))?;")
            .unwrap()
            < store.find("*guard = Some(profile);").unwrap(),
        "{store}"
    );
    let apply = code_of(&production, "pub(crate) async fn apply_session(");
    assert!(
        apply.contains("let email = session_email(profile.as_ref());"),
        "{apply}"
    );
    assert!(
        apply.find("session_email(").unwrap() < apply.find("install_new_session_or_revoke(").unwrap(),
        "the server's email reaches the checked install"
    );
    assert!(
        code_of(&production, "async fn install_new_session_or_revoke(")
            .contains("install_new_session(state, acct, turn, token, master_key, email, profile)?")
    );
    assert!(
        code_of(&production, "fn install_new_session(")
            .contains("persist_session_to_keychain(&write, acct.id.as_str(), token, master_key, email.as_deref())?;")
    );
    let signature = apply[..apply.find(") -> Result<(), String> {").unwrap()].to_string();
    assert!(
        !signature.contains("email"),
        "apply_session takes no email, only the server's profile:\n{signature}"
    );
    let browser = include_str!("../browser_login.rs");
    assert!(
        browser.contains("crate::fetch_canonical_profile(&creds.session_token).await"),
        "the browser handoff fetches the profile"
    );
    assert!(
        browser.contains("let email = crate::session_email(profile.as_ref());"),
        "and names the email from it alone"
    );
    assert!(!browser.contains("creds.email"), "the browser's own email is not read");
}

/// Fix round 3 (R2 residual): a sign-in whose account-record fetch FAILED has no identity, and an unidentified
/// session starts nothing and changes nothing: no engine, no purge, no adoption, no owner recorded, no Finder
/// debt, no database opened. Whatever the local data is: another account's, unowned, or none.
#[tokio::test]
async fn an_unidentified_session_starts_nothing_and_changes_nothing() {
    let unidentified = signed_in_via(None); // what `fetch_canonical_profile` -> None leaves behind
    assert!(
        unidentified
            .acct
            .session
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|session| session.email.is_none())
    );
    for (name, owner) in [
        ("another account's data", Some(alice())),
        ("data with no recorded owner", None),
    ] {
        let local = local_data_of(owner.clone());
        let mut slot = unidentified.acct.engine.try_lock().unwrap();
        let paths = LocalDataPaths::for_test(local.dir.path(), &local.staging);
        let created = std::cell::Cell::new(false);
        let result = start_engine_bound(
            &unidentified.state,
            &unidentified.acct,
            &mut slot,
            &paths,
            |_root, _token, _key| {
                created.set(true);
                crate::runner::EngineRunner::for_test_with_task(tokio::spawn(async {}))
            },
        );
        assert_eq!(
            result,
            Err(crate::account_binding::IDENTITY_UNKNOWN.to_string()),
            "{name}: the existing identity message"
        );
        assert!(!created.get() && slot.is_none(), "{name}: no engine");
        assert_eq!(
            local.data.db.list_due_operations(i64::MAX).unwrap().len(),
            1,
            "{name}: nothing purged"
        );
        for path in &local.files {
            assert!(path.exists(), "{name}: {} is kept", path.display());
        }
        assert_eq!(
            local.data.db.owner().unwrap(),
            owner,
            "{name}: no owner recorded, none adopted, none replaced"
        );
        assert!(!local.data.db.finder_removal_owed().unwrap(), "{name}: no Finder debt");
    }
    // No data at all: the database is not even created.
    let dir = tempfile::tempdir().unwrap();
    let mut slot = unidentified.acct.engine.try_lock().unwrap();
    let result = start_engine_bound(
        &unidentified.state,
        &unidentified.acct,
        &mut slot,
        &LocalDataPaths::for_test(dir.path(), dir.path()),
        |_root, _token, _key| panic!("no engine for an unidentified session"),
    );
    assert_eq!(result, Err(crate::account_binding::IDENTITY_UNKNOWN.to_string()));
    assert!(
        !dir.path().join("state.db").exists(),
        "an unidentified start opens nothing"
    );
}

/// Fix round 3: the fetch fails, nothing happens; a later fetch that succeeds names the session, and the start
/// then binds exactly as any other (the same account keeps its queue, another account's is reset).
#[tokio::test]
async fn a_later_fetch_that_succeeds_names_the_session_and_the_start_binds_normally() {
    for (name, profile, is_the_owner) in [
        ("the owner", profile_of("u-a", "a@beebeeb.io"), true),
        ("another account", profile_of("u-b", "b@beebeeb.io"), false),
    ] {
        let local = local_data_of(Some(alice()));
        let fx = signed_in_via(None);
        let paths = LocalDataPaths::for_test(local.dir.path(), &local.staging);
        // The account record cannot be fetched (the server errors, or answers with something that is no
        // profile): still unidentified, still refused, nothing deleted.
        for (status, body) in [("500 Internal Server Error", "{}"), ("200 OK", "{}")] {
            let base_url = answering(status, body.to_string());
            assert!(
                !establish_session_identity(&fx.state, &fx.acct, &base_url, None).await,
                "{name}: {status}"
            );
            assert_eq!(
                fx.acct.session.lock().unwrap().as_ref().unwrap().email,
                None,
                "{name}: still unidentified"
            );
            let slot = fx.acct.engine.try_lock().unwrap();
            let refused = authorize_engine_start(&fx.state, &fx.acct, &slot, &paths);
            assert_eq!(
                refused.err(),
                Some(crate::account_binding::IDENTITY_UNKNOWN.to_string()),
                "{name}: {status}"
            );
            assert_eq!(
                local.data.db.list_due_operations(i64::MAX).unwrap().len(),
                1,
                "{name}: nothing purged"
            );
            assert_eq!(
                local.data.db.owner().unwrap(),
                Some(alice()),
                "{name}: the owner is untouched"
            );
        }
        // A later fetch succeeds.
        let base_url = answering("200 OK", serde_json::to_string(&profile).unwrap());
        assert!(
            establish_session_identity(&fx.state, &fx.acct, &base_url, None).await,
            "{name}"
        );
        assert_eq!(
            fx.acct.session.lock().unwrap().as_ref().unwrap().email.as_deref(),
            Some(profile.email.as_str()),
            "{name}: named by the server"
        );
        assert_eq!(
            fx.acct
                .cached_profile
                .lock()
                .unwrap()
                .as_ref()
                .map(|cached| cached.user_id.clone()),
            Some(profile.user_id.clone())
        );
        assert_eq!(
            fx.acct.auth_email.lock().unwrap().as_deref(),
            Some(profile.email.as_str()),
            "{name}: the Account page shows it"
        );
        let slot = fx.acct.engine.try_lock().unwrap();
        let result = authorize_after_removal(&fx, &slot, &paths);
        if is_the_owner {
            assert!(matches!(result, Ok(StartPermit::Go(_))), "{name}: {result:?}");
            assert_eq!(
                local.data.db.list_due_operations(i64::MAX).unwrap().len(),
                1,
                "{name}: its own queue is kept"
            );
            assert_eq!(local.data.db.owner().unwrap(), Some(alice()));
        } else if RESET_ALLOWED {
            assert!(matches!(result, Ok(StartPermit::Go(_))), "{name}: {result:?}");
            assert!(
                local.data.db.list_due_operations(i64::MAX).unwrap().is_empty(),
                "{name}: another account's queue is reset"
            );
            assert_eq!(local.data.db.owner().unwrap(), Some(bob()));
        } else {
            assert_eq!(
                result.err(),
                Some(crate::account_binding::OTHER_ACCOUNT_ON_WINDOWS.to_string())
            );
        }
        // A session that is named already, or was replaced meanwhile, is left alone.
        assert!(
            !establish_session_identity(&fx.state, &fx.acct, &base_url, None).await,
            "{name}: already named"
        );
    }
}

/// A signed-in session whose stored email is a legacy spelling (`Sam@Beebeeb.io`, as an older build saved it) and the
/// local data it owns under that same spelling, with no id recorded; the server spells the address `sam@beebeeb.io`.
pub(super) fn a_legacy_spelling() -> (AuthorizeFixture, LocalFixture, account_dto::AccountProfile) {
    let legacy = Identity::new(None, Some("Sam@Beebeeb.io"));
    let fx = AuthorizeFixture::with_session(&legacy);
    AuthVault::new(platform_keychain_store_for(fx.acct.id.as_str()))
        .store_account_email("Sam@Beebeeb.io")
        .ok();
    (fx, local_data_of(Some(legacy)), profile_of("u-1", "sam@beebeeb.io"))
}

/// Ruling A″ (fix round 3): the owner backfill and the naming compare in the engine start's own canonical form, so a
/// legacy spelling that differs from the server's only in a NON-ASCII letter case (`Émile@…`, `émile@…`) is
/// completed and named like an ASCII variant. Both folded ASCII letters only before.
#[test]
fn a_non_ascii_legacy_spelling_is_completed_and_named() {
    let legacy = Identity::new(None, Some("Émile@beebeeb.io"));
    let fx = AuthorizeFixture::with_session(&legacy);
    let local = local_data_of(Some(legacy));
    let server = profile_of("u-1", "émile@beebeeb.io");
    assert_eq!(
        backfill_local_data_owner(&SessionWrite::for_test(), local.dir.path(), &server),
        Ok(true),
        "the owner is completed"
    );
    assert_eq!(
        local.data.db.owner().unwrap(),
        Some(Identity::new(Some("u-1"), Some("émile@beebeeb.io")))
    );
    let named = name_session_after_profile(
        &SessionWrite::for_test(),
        &fx.state,
        &fx.acct,
        "tok-r10",
        &server,
        Some(local.dir.path()),
    );
    assert!(named, "the session is named after the server's spelling");
    assert_eq!(session_spelling(&fx).as_deref(), Some("émile@beebeeb.io"));
    assert_eq!(
        local.data.db.list_due_operations(i64::MAX).unwrap().len(),
        1,
        "its queued change is kept"
    );
}

pub(super) fn session_spelling(fx: &AuthorizeFixture) -> Option<String> {
    fx.acct
        .session
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|session| session.email.clone())
}

/// Fix round 1, item 1 (review I1, P3-A): the session is renamed to the server's spelling only after the
/// owner record was completed. When that fails (here the state dir holds a `state.db` that is not a database, so it
/// cannot be read, on every platform), nothing is renamed: the session, the Keychain email and the cache stay as
/// they were, and the next engine start still knows the account's own data (its queued change is kept, nothing is
/// reset).
#[test]
fn a_failed_owner_backfill_renames_nothing_and_the_next_start_keeps_the_data() {
    let (fx, local, server) = a_legacy_spelling();
    let keychain_before = keychain_account_email(fx.acct.id.as_str());
    let unreadable = tempfile::tempdir().unwrap();
    std::fs::write(
        unreadable.path().join(crate::state_paths::STATE_DB_FILENAME),
        b"this is not a database, and it is long enough to be read as one: 0123456789",
    )
    .unwrap();
    assert!(
        state_db_from_state_dir(unreadable.path()).is_err(),
        "the premise: state.db cannot be read"
    );
    let named = name_session_after_profile(
        &SessionWrite::for_test(),
        &fx.state,
        &fx.acct,
        "tok-r10",
        &server,
        Some(unreadable.path()),
    );
    assert!(!named, "a failed backfill names nothing");
    assert_eq!(
        session_spelling(&fx).as_deref(),
        Some("Sam@Beebeeb.io"),
        "the stored spelling stays"
    );
    assert_eq!(
        keychain_account_email(fx.acct.id.as_str()),
        keychain_before,
        "and so does the Keychain's"
    );
    assert!(fx.acct.cached_profile.lock().unwrap().is_none(), "nothing is cached");
    let session = identity_of_session(
        session_spelling(&fx).as_deref(),
        fx.acct.cached_profile.lock().unwrap().as_ref(),
    );
    assert_eq!(
        bind_before_engine_start(&local.data, &session, RESET_ALLOWED),
        Ok(Bound::Kept),
        "the next start keeps the data"
    );
    assert_eq!(
        local.data.db.list_due_operations(i64::MAX).unwrap().len(),
        1,
        "its queued change is still there"
    );
}

/// Fix round 2, item 3 (re-check N2): the naming completed the owner record (its id and the server's
/// spelling) but could not write the Keychain email, and the next launch is OFFLINE: the session comes back from the
/// Keychain with the legacy spelling and no id. The engine-start binding must not take that for another account:
/// nothing of the account's own data is reset. Since ruling A″ the spelling IS the same account (equal in canonical
/// form), so the offline start proceeds and keeps everything; under rulings A and A′ it was refused until a fetch.
#[cfg(target_os = "macos")]
#[test]
fn a_stale_keychain_spelling_after_a_completed_owner_never_resets_the_data_on_an_offline_launch() {
    let (fx, local, server) = a_legacy_spelling();
    let id = fx.acct.id.as_str().to_string();
    // The naming in the run before: the backfill succeeds, the Keychain email write fails.
    crate::keychain::test_fail_account_email_writes(Some(&id), true);
    let named = name_session_after_profile(
        &SessionWrite::for_test(),
        &fx.state,
        &fx.acct,
        "tok-r10",
        &server,
        Some(local.dir.path()),
    );
    crate::keychain::test_fail_account_email_writes(Some(&id), false);
    assert!(named, "the premise: the backfill succeeded, so the session was named");
    assert_eq!(
        local.data.db.owner().unwrap(),
        Some(Identity::new(Some("u-1"), Some("sam@beebeeb.io"))),
        "the premise: the owner record was completed"
    );
    assert_eq!(
        keychain_account_email(&id).as_deref(),
        Some("Sam@Beebeeb.io"),
        "the premise: the Keychain write failed"
    );
    // The offline launch: the session comes back from the Keychain, no profile can be fetched.
    let restored = identity_of_session(keychain_account_email(&id).as_deref(), None);
    assert_eq!(
        restored,
        Identity::new(None, Some("Sam@Beebeeb.io")),
        "the premise: the stale spelling, no id"
    );
    assert_eq!(
        bind_before_engine_start(&local.data, &restored, RESET_ALLOWED),
        Ok(Bound::Kept),
        "the same account starts offline; its own data is never reset for a spelling"
    );
    assert_eq!(
        local.data.db.list_due_operations(i64::MAX).unwrap().len(),
        1,
        "its queued change is still there"
    );
    assert_eq!(
        local.data.db.owner().unwrap().and_then(|owner| owner.user_id),
        Some("u-1".to_string()),
        "and its owner record keeps the account's id"
    );
    // Once a fetch names the session (the cached profile carries the id), the ids decide and the data is kept.
    let named_again = identity_of_session(Some("sam@beebeeb.io"), Some(&server));
    assert_eq!(
        bind_before_engine_start(&local.data, &named_again, RESET_ALLOWED),
        Ok(Bound::Kept)
    );
    assert_eq!(
        local.data.db.list_due_operations(i64::MAX).unwrap().len(),
        1,
        "still there"
    );
}
