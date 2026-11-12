//! Split out of `account_binding_tests.rs` for the file-size ratchet; the tests are unchanged.

use super::part_2::*;
use super::*;

/// Ruling P (Task 12 fix round 2): a session that does not know who it is (its account record could not be fetched)
/// is refused, and `sync_status` carries the closed code and its sentence and nothing else. The later naming start
/// (the session named after the server's record and its profile cached, as `name_session_after_profile` leaves it)
/// runs and clears it. Fix round 3: this used a case-only spelling, which ruling A″ makes the same account.
#[test]
fn an_unidentified_sessions_refusal_shows_in_sync_status_until_the_naming_start_runs() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let local = local_data_of(Some(Identity::new(Some("u-1"), Some("sam@beebeeb.io"))));
        let fx = AuthorizeFixture::with_session(&Identity::default());
        let paths = LocalDataPaths::for_test(local.dir.path(), &local.staging);
        let mut slot = fx.acct.engine.lock().await;
        let refused = start_engine_bound(&fx.state, &fx.acct, &mut slot, &paths, |_root, _token, _key| {
            unreachable!("no engine")
        });
        assert_eq!(
            refused,
            Err(crate::account_binding::IDENTITY_UNKNOWN.to_string()),
            "the premise: the start is refused"
        );
        let shown = engine_refusal_view(&fx.acct);
        assert_eq!(
            shown,
            serde_json::json!({ "code": "identity_unknown", "sentence": crate::account_binding::IDENTITY_UNKNOWN })
        );
        assert_eq!(
            shown.as_object().unwrap().len(),
            2,
            "the code and the sentence, nothing about the account"
        );
        fx.acct.session.lock().unwrap().as_mut().unwrap().email = Some("sam@beebeeb.io".into());
        *fx.acct.cached_profile.lock().unwrap() = Some(profile_of("u-1", "sam@beebeeb.io"));
        let started = start_engine_bound(&fx.state, &fx.acct, &mut slot, &paths, |_root, _token, _key| {
            crate::runner::EngineRunner::for_test_with_task(tokio::spawn(async {}))
        });
        assert_eq!(started, Ok(EngineStart::Started), "the premise: the naming start runs");
        assert_eq!(
            engine_refusal_view(&fx.acct),
            serde_json::Value::Null,
            "and clears the refusal"
        );
        assert_eq!(
            local.data.db.list_due_operations(i64::MAX).unwrap().len(),
            1,
            "the account's own change is kept"
        );
    });
}

/// Fix round 3, M5: a Lock and a Sign-out drop a Keychain email write still owed (it belonged to the session they
/// end); a Write keeps it, because the naming that marks a failed write moves the generation with a Write.
#[test]
fn a_lock_and_a_sign_out_drop_a_keychain_email_write_still_owed_and_a_write_keeps_it() {
    use crate::account::SessionTransition;
    let fx = AuthorizeFixture::with_session(&alice());
    for (by, kept) in [
        (SessionTransition::Lock, false),
        (SessionTransition::SignOut, false),
        (SessionTransition::Write, true),
    ] {
        fx.acct.keychain_email_pending.store(true, Ordering::SeqCst);
        let _turn = SessionWrite::for_test();
        fx.acct.advance_session_generation_holding_session_write_lock(by);
        assert_eq!(fx.acct.keychain_email_pending.load(Ordering::SeqCst), kept, "{by:?}");
    }
}

/// Ruling P: an engine-start error that is not one of the two binding refusals records nothing (here the slot is
/// taken), and only the exact two sentences count as refusals.
#[test]
fn an_engine_start_error_that_is_not_a_binding_refusal_records_nothing() {
    use crate::account_binding::{IDENTITY_UNKNOWN, OTHER_ACCOUNT_ON_WINDOWS, Refusal};
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let local = local_data_of(Some(alice()));
        let fx = AuthorizeFixture::with_session(&alice());
        let paths = LocalDataPaths::for_test(local.dir.path(), &local.staging);
        let mut slot = fx.acct.engine.lock().await;
        *slot = Some(crate::runner::EngineRunner::for_test_with_task(tokio::spawn(async {})));
        let refused = start_engine_bound(&fx.state, &fx.acct, &mut slot, &paths, |_root, _token, _key| {
            unreachable!("no engine")
        });
        assert_eq!(
            refused,
            Err(ENGINE_SLOT_OCCUPIED_ERROR.to_string()),
            "the premise: another error"
        );
        assert_eq!(
            engine_refusal_view(&fx.acct),
            serde_json::Value::Null,
            "recorded nothing"
        );
    });
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

/// Ruling P: every session transition clears the refusal, and a refusal found before a transition is not recorded
/// after it (the start checks the generation it read, under the same mutex the transition clears).
#[test]
fn every_session_transition_clears_the_engine_refusal_and_a_stale_one_is_not_recorded() {
    use crate::account::SessionTransition;
    use crate::account_binding::{IDENTITY_UNKNOWN, Refusal};
    let fx = AuthorizeFixture::with_session(&alice());
    for by in [
        SessionTransition::Write,
        SessionTransition::Lock,
        SessionTransition::SignOut,
    ] {
        *fx.acct.engine_refusal.lock().unwrap() = Some(Refusal::IdentityUnknown);
        let _turn = SessionWrite::for_test();
        fx.acct.advance_session_generation_holding_session_write_lock(by);
        assert_eq!(
            engine_refusal_view(&fx.acct),
            serde_json::Value::Null,
            "{by:?} cleared it"
        );
    }
    let before = fx.acct.session_generation();
    {
        let _turn = SessionWrite::for_test();
        fx.acct
            .advance_session_generation_holding_session_write_lock(SessionTransition::Write);
    }
    record_engine_refusal(&fx.acct, before, IDENTITY_UNKNOWN);
    assert_eq!(
        engine_refusal_view(&fx.acct),
        serde_json::Value::Null,
        "found before the transition: not recorded"
    );
    record_engine_refusal(&fx.acct, fx.acct.session_generation(), IDENTITY_UNKNOWN);
    assert_eq!(
        engine_refusal_view(&fx.acct)["code"],
        "identity_unknown",
        "found for the current session: recorded"
    );
}

/// Local data with an owner and a file row (which writes the Finder change log too), nothing waiting to upload:
/// the state the Windows sign-out sees for an account that synced and finished.
fn a_synced_account(owner: &Identity) -> (tempfile::TempDir, StateDbLocalData) {
    let dir = tempfile::tempdir().unwrap();
    let db = crate::state_db::StateDb::open(dir.path().join("state.db")).unwrap();
    db.upsert_file(&crate::state_db::FileEntry {
        file_id: "file-a".into(),
        path: "/A.txt".into(),
        status: crate::state_db::FileStatus::Local,
        size_bytes: 11,
        modified_at: 0,
        content_hash: None,
        remote_updated_at: 0,
        parent_id: None,
        item_kind: crate::state_db::ItemKind::File,
    })
    .unwrap();
    db.set_owner(owner).unwrap();
    (
        dir,
        StateDbLocalData {
            db,
            sync_root: None,
            staging_dirs: Vec::new(),
            kept: Default::default(),
        },
    )
}

/// The database steps of the Windows sign-out (`windows_cf::signout::purge`; its file handling is Windows-only).
fn windows_sign_out(data: &StateDbLocalData) {
    data.db
        .windows_signout_preflight()
        .expect("nothing waits to upload, so the sign-out goes ahead");
    data.db.finish_windows_signout().expect("and finishes");
}

/// Fix to the R10 binding (ruling W1): after a Windows sign-out of an account that had a file row, nothing of that
/// account counts as local data any more, so another account's sign-in starts instead of being refused as if the
/// previous account's data were still here.
#[test]
fn after_a_windows_sign_out_another_account_starts() {
    let (_dir, data) = a_synced_account(&alice());
    windows_sign_out(&data);
    assert!(!data.db.has_account_data().unwrap(), "no account data is left");
    assert_eq!(data.db.owner().unwrap(), None, "and no owner");
    assert_eq!(
        bind_before_engine_start(&data, &bob(), false),
        Ok(Bound::Kept),
        "another account's sign-in starts"
    );
    assert_eq!(
        data.db.owner().unwrap(),
        Some(bob()),
        "and owns what it syncs from now on"
    );
}

/// Ruling A″ (fix round 3): on Windows a letter-case variant of the owner's email starts without a sign-out, as it
/// did before the binding existed (ruling A′ refused it with "Sign out, then sign in again"). Its synced data is
/// kept, and the record follows the session's spelling.
#[test]
fn on_windows_a_case_variant_starts_without_a_sign_out() {
    let (_dir, data) = a_synced_account(&Identity::new(None, Some("sam@beebeeb.io")));
    let variant = Identity::new(None, Some("Sam@Beebeeb.io"));
    assert_eq!(
        bind_before_engine_start(&data, &variant, false),
        Ok(Bound::Kept),
        "it starts"
    );
    assert!(data.db.has_account_data().unwrap(), "and its data is kept");
    assert_eq!(
        data.db.owner().unwrap(),
        Some(variant),
        "one owner, in the session's spelling"
    );
}

/// Ruling W2 (kept as it is): with a change waiting to upload, the Windows sign-out refuses ("Pending changes
/// remain…") and discards nothing, and the binding keeps refusing a start for an email that is not the owner's
/// (here the same person's changed address, which no id settles). Fix round 3: this used a case-only variant, which
/// ruling A″ makes the same account.
#[test]
fn on_windows_a_change_waiting_to_upload_blocks_the_sign_out_and_the_start_stays_refused() {
    let local = local_data_of(Some(Identity::new(None, Some("sam@beebeeb.io"))));
    let changed_address = Identity::new(None, Some("samuel@beebeeb.io"));
    let refused = Err(crate::account_binding::OTHER_ACCOUNT_ON_WINDOWS.to_string());
    assert_eq!(
        bind_before_engine_start(&local.data, &changed_address, false),
        refused,
        "the premise"
    );
    assert!(
        local.data.db.windows_signout_preflight().is_err(),
        "the sign-out refuses while a change waits"
    );
    assert!(
        local.data.db.finish_windows_signout().is_err(),
        "and could not finish either"
    );
    assert_eq!(
        bind_before_engine_start(&local.data, &changed_address, false),
        refused,
        "so the next start is refused again"
    );
    assert_eq!(
        local.data.db.list_due_operations(i64::MAX).unwrap().len(),
        1,
        "nothing was discarded"
    );
}

/// Fix round 1, item 1 (review I1, P2-B): with no owner record to complete (Windows), a session that has an
/// email is never renamed to another spelling, so an upgraded Windows user whose saved email differs from the
/// server's only in letter case starts normally (the profile the restore caches anyway does not change who the
/// session is: it names a different spelling). Since ruling A″ the renamed session would start too (the same
/// canonical address); under rulings A and A′ it was refused.
#[test]
fn without_an_owner_record_a_case_variant_email_is_never_renamed_and_starts_normally() {
    let (fx, local, server) = a_legacy_spelling();
    let named = name_session_after_profile(&SessionWrite::for_test(), &fx.state, &fx.acct, "tok-r10", &server, None);
    assert!(!named);
    assert_eq!(
        session_spelling(&fx).as_deref(),
        Some("Sam@Beebeeb.io"),
        "never renamed"
    );
    *fx.acct.cached_profile.lock().unwrap() = Some(server.clone()); // the restore caches the probed profile either way
    let session = identity_of_session(
        session_spelling(&fx).as_deref(),
        fx.acct.cached_profile.lock().unwrap().as_ref(),
    );
    assert_eq!(
        bind_before_engine_start(&local.data, &session, false),
        Ok(Bound::Kept),
        "Windows starts normally"
    );
    let renamed = identity_of_session(Some("sam@beebeeb.io"), Some(&server));
    assert_eq!(
        bind_before_engine_start(&local.data, &renamed, false),
        Ok(Bound::Kept),
        "a renamed session starts too"
    );
    // An email-less session is still named, as before (that is not a rename).
    let unnamed = signed_in_via(None);
    assert!(name_session_after_profile(
        &SessionWrite::for_test(),
        &unnamed.state,
        &unnamed.acct,
        "tok-r10",
        &server,
        None
    ));
    assert_eq!(session_spelling(&unnamed).as_deref(), Some("sam@beebeeb.io"));
}

/// Fix round 3: an answer is never applied to a session it was not fetched for. If the session is replaced (a
/// different token) or named by someone else while the fetch is in flight, the old answer is dropped, so one
/// account's record can never become the identity of another account's keys.
#[tokio::test]
async fn an_answer_is_dropped_when_the_session_changed_while_the_fetch_ran() {
    let owners_answer = serde_json::to_string(&profile_of("u-a", "a@beebeeb.io")).unwrap();
    // 1. The session is replaced by another sign-in's (another token) during the fetch.
    let fx = signed_in_via(None);
    let acct = fx.acct.clone();
    let base_url = answering_after("200 OK", owners_answer.clone(), move || {
        *acct.session.lock().unwrap() = Some(Session {
            token: "tok-another".into(),
            master_key: [9u8; 32],
            email: None,
        });
    });
    assert!(
        !establish_session_identity(&fx.state, &fx.acct, &base_url, None).await,
        "a replaced session is not named by the old answer"
    );
    {
        let session = fx.acct.session.lock().unwrap();
        assert_eq!(
            session.as_ref().map(|s| (s.token.as_str(), s.email.clone())),
            Some(("tok-another", None)),
            "the new session stays unidentified"
        );
    }
    assert!(
        fx.acct.cached_profile.lock().unwrap().is_none(),
        "and no profile is cached for it"
    );
    // 2. The same session is named by someone else during the fetch: the first name stands.
    let fx = signed_in_via(None);
    let acct = fx.acct.clone();
    let base_url = answering_after("200 OK", owners_answer, move || {
        acct.session.lock().unwrap().as_mut().unwrap().email = Some("someone-else@beebeeb.io".into());
    });
    assert!(!establish_session_identity(&fx.state, &fx.acct, &base_url, None).await);
    assert_eq!(
        fx.acct.session.lock().unwrap().as_ref().unwrap().email.as_deref(),
        Some("someone-else@beebeeb.io")
    );
    assert!(fx.acct.cached_profile.lock().unwrap().is_none());
}

/// Fix round 3: the restore at launch names an unidentified session BEFORE it asks an engine to start, so the
/// "open Beebeeb again" the refusal asks for is enough.
#[test]
fn the_startup_restore_names_an_unidentified_session_before_it_starts_an_engine() {
    let production = production_source();
    // Task 12 fix round 1, item 7: the restore itself asks the server once (the probe, whose answer names the
    // session); a second request runs in the background after the engine start, which never waits for it.
    let restore = code_of(&production, "async fn restore_stored_session(");
    assert!(
        !restore.contains("establish_session_identity("),
        "the restore does not wait for a second request:\n{restore}"
    );
    let inner = code_of(&production, "async fn restore_session_inner(");
    let restored = inner
        .find("if !restore_stored_session(&state, &acct, &runner::api_base_url()).await {")
        .expect("the restore runs first");
    let started = inner.find("start_engine_if_possible(").expect("and then starts");
    let spawned = inner
        .find("tauri::async_runtime::spawn(identify_then_start(app.clone(), IdentifyPace::Now));")
        .expect("the second request is spawned");
    assert!(restored < started && started < spawned, "{inner}");
    let windows = inner
        .find("#[cfg(target_os = \"windows\")]\n    if acct\n        .session\n        .lock()")
        .expect("Windows keeps today's rule");
    assert!(
        inner[windows..started]
            .contains("establish_session_identity(&state, &acct, &runner::api_base_url(), None).await;"),
        "{inner}"
    );
    assert!(
        inner[spawned - 60..spawned].contains("#[cfg(not(target_os = \"windows\"))]"),
        "{inner}"
    );
}

/// R3 (fix round 2): a start onto an UNOWNED, EMPTY database on a Mac owes the Finder removal as a reset does: the
/// database may have been deleted or never existed while the previous account's domain stayed registered. The
/// debt is recorded with the first owner, nothing starts in that call, and a confirmed removal (or an Observe that
/// finds no domain) lets the next start through. Off a Mac nothing is owed.
#[test]
fn a_first_start_on_an_unowned_empty_database_owes_the_finder_removal_on_a_mac() {
    let fx = AuthorizeFixture::with_session(&alice());
    let dir = tempfile::tempdir().unwrap();
    let slot = fx.acct.engine.try_lock().unwrap();
    let paths = LocalDataPaths::for_test(dir.path(), dir.path());
    let first = authorize_engine_start(&fx.state, &fx.acct, &slot, &paths);
    let db = crate::state_db::StateDb::open(dir.path().join("state.db")).unwrap();
    assert_eq!(
        db.owner().unwrap(),
        Some(alice()),
        "the first account to start here is recorded"
    );
    if cfg!(target_os = "macos") {
        assert!(
            matches!(first, Ok(StartPermit::FinderRemovalOwed)),
            "nothing starts while a removal is owed: {first:?}"
        );
        assert!(db.finder_removal_owed().unwrap(), "the debt is recorded");
        #[cfg(target_os = "macos")]
        clear_finder_removal_owed_in(dir.path());
        let second = authorize_engine_start(&fx.state, &fx.acct, &slot, &paths);
        assert!(
            matches!(second, Ok(StartPermit::Go(_))),
            "a confirmed removal lets the start through: {second:?}"
        );
        assert!(
            !db.finder_removal_owed().unwrap(),
            "and nothing is owed again: the owner is recorded now"
        );
    } else {
        assert!(matches!(first, Ok(StartPermit::Go(_))), "no Finder, no debt: {first:?}");
        assert!(!db.finder_removal_owed().unwrap());
    }
}

/// R3: the exceptions are exact. A sign-out whose Finder removal was CONFIRMED leaves the domain gone, so the
/// next account's first start owes nothing (no wait on every sign-in after a sign-out); one whose removal was
/// not confirmed owes it. Off a Mac there is no Finder domain and no removal to confirm: a start after any
/// sign-out owes nothing.
#[test]
fn a_start_after_a_confirmed_sign_out_owes_nothing_and_after_an_unconfirmed_one_it_does() {
    for (owe_after_sign_out, expect_debt) in [(false, false), (true, true)] {
        let local = local_data_of(Some(alice()));
        // The sign-out purge, as `clear_session_impl` runs it, with its removal confirmed or not. Off a Mac it
        // passes `owe_removal = false` whatever happened (pinned by
        // `a_failed_sign_out_purge_stops_the_sign_out_on_macos_and_linux`).
        #[cfg(not(target_os = "windows"))]
        assert_eq!(
            with_own_config(|| purge_local_data_for_sign_out(
                Ok(local.dir.path().to_path_buf()),
                None,
                cfg!(target_os = "macos") && owe_after_sign_out,
                None
            )),
            Ok(())
        );
        // Windows signs out through `windows_cf::signout::purge`, which refuses while a change is queued (so the
        // queue is emptied first, as a finished sync leaves it) and ends in `finish_windows_signout`: every row
        // cleared, no Finder debt recorded.
        #[cfg(target_os = "windows")]
        {
            local.data.db.purge_all_local_state().unwrap();
            local.data.db.finish_windows_signout().unwrap();
        }
        let fx = AuthorizeFixture::with_session(&bob());
        let slot = fx.acct.engine.try_lock().unwrap();
        let result = authorize_engine_start(
            &fx.state,
            &fx.acct,
            &slot,
            &LocalDataPaths::for_test(local.dir.path(), &local.staging),
        );
        if cfg!(target_os = "macos") && expect_debt {
            assert!(
                matches!(result, Ok(StartPermit::FinderRemovalOwed)),
                "owe = {owe_after_sign_out}: {result:?}"
            );
        } else {
            assert!(
                matches!(result, Ok(StartPermit::Go(_))),
                "owe = {owe_after_sign_out}: {result:?}"
            );
            assert!(!local.data.db.finder_removal_owed().unwrap());
        }
    }
}

/// Fix round 3: the "domain gone" mark holds only until the domain is registered again. After a confirmed
/// sign-out the next first start owes nothing; once the reconciler has added the domain the mark is cleared, so
/// an unowned, empty database found later owes the removal check again.
#[cfg(target_os = "macos")]
#[test]
fn a_registered_domain_is_no_longer_known_gone_so_a_later_first_start_owes_a_check() {
    let local = local_data_of(Some(alice()));
    assert_eq!(
        with_own_config(|| purge_local_data_for_sign_out(Ok(local.dir.path().to_path_buf()), None, false, None)),
        Ok(())
    );
    assert!(
        local.data.db.finder_domain_gone().unwrap(),
        "a confirmed sign-out leaves the domain known gone"
    );
    // The reconciler adds the domain (a confirmed add), as `Ports::domain_added` does on a Mac.
    clear_finder_domain_gone_in(local.dir.path());
    assert!(
        !local.data.db.finder_domain_gone().unwrap(),
        "a confirmed add clears the mark"
    );
    // The owner row is gone with the sign-out and the database is empty: the next first start owes the check.
    let fx = AuthorizeFixture::with_session(&bob());
    let slot = fx.acct.engine.try_lock().unwrap();
    let result = authorize_engine_start(
        &fx.state,
        &fx.acct,
        &slot,
        &LocalDataPaths::for_test(local.dir.path(), &local.staging),
    );
    assert!(matches!(result, Ok(StartPermit::FinderRemovalOwed)), "{result:?}");
}

/// R4 (fix round 2): the staging directories the sweep covers come from the PRODUCTION call sites, and they pass
/// the complete list. A test build's list is empty unless a test sets it (so no test sweeps the shared sandbox the
/// engine-bridge tests stage into); these tests set it and drive the real call sites, so a call site that passes an
/// empty or a shortened list leaves an orphan behind and fails.
struct StagingOverride {
    _serial: std::sync::MutexGuard<'static, ()>,
}

impl StagingOverride {
    fn set(dirs: Vec<PathBuf>) -> Self {
        static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        *STAGING_DIRS_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner()) = Some(dirs);
        Self { _serial: serial }
    }
}

impl Drop for StagingOverride {
    fn drop(&mut self) {
        *STAGING_DIRS_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

/// Two staging directories in the shape the engine uses (`<root>/beebeeb/finder-writes`), one orphan in each.
fn two_staging_dirs_with_orphans() -> (PathBuf, Vec<PathBuf>, Vec<PathBuf>) {
    let root = std::env::temp_dir().join(format!("bb-test-r4-{}", uuid::Uuid::new_v4()));
    let dirs: Vec<PathBuf> = ["cache", "temp"]
        .iter()
        .map(|name| root.join(name).join("beebeeb").join("finder-writes"))
        .collect();
    let orphans: Vec<PathBuf> = dirs.iter().map(|dir| dir.join("orphan.bin")).collect();
    for (dir, orphan) in dirs.iter().zip(&orphans) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(orphan, b"plaintext with no row").unwrap();
    }
    (root, dirs, orphans)
}

#[cfg(not(target_os = "windows"))]
#[test]
fn the_sign_out_purge_sweeps_every_production_staging_directory() {
    let (root, dirs, orphans) = two_staging_dirs_with_orphans();
    let _override = StagingOverride::set(dirs);
    let state_dir = tempfile::tempdir().unwrap();
    crate::state_db::StateDb::open(state_dir.path().join("state.db")).unwrap();
    // The function `clear_session_impl` calls, which reaches `purge_local_state_files`.
    assert_eq!(
        with_own_config(|| purge_local_data_for_sign_out(Ok(state_dir.path().to_path_buf()), None, false, None)),
        Ok(())
    );
    let left: Vec<&PathBuf> = orphans.iter().filter(|orphan| orphan.exists()).collect();
    let _ = std::fs::remove_dir_all(&root);
    assert!(
        left.is_empty(),
        "every staging directory is swept, none is skipped: {left:?}"
    );
}

#[test]
fn the_engine_start_reset_sweeps_every_production_staging_directory() {
    if !RESET_ALLOWED {
        return; // Windows refuses instead of resetting
    }
    let (root, dirs, orphans) = two_staging_dirs_with_orphans();
    let _override = StagingOverride::set(dirs.clone());
    crate::state_paths::init_for_test(); // the process-wide state dir `LocalDataPaths::production` resolves
    let mut paths = LocalDataPaths::production(&root).unwrap();
    assert_eq!(paths.staging_dirs, dirs, "the production paths carry the complete list");
    // The real paths, but with a database of this test's own (the shared one belongs to the sign-out tests).
    let local = local_data_of(Some(alice()));
    paths.state_dir = local.dir.path().to_path_buf();
    let fx = AuthorizeFixture::with_session(&bob());
    let slot = fx.acct.engine.try_lock().unwrap();
    let _ = authorize_engine_start(&fx.state, &fx.acct, &slot, &paths);
    let left: Vec<&PathBuf> = orphans.iter().filter(|orphan| orphan.exists()).collect();
    let _ = std::fs::remove_dir_all(&root);
    assert!(left.is_empty(), "the reset swept every staging directory: {left:?}");
}

/// R4 (fix round 3): the list a RELEASE build sweeps is complete. A test build never compiles the release arm of
/// `production_staging_dirs` (it reads the override), so two things hold it: the arm is exactly one call to
/// `release_staging_dirs` (source pin), and that function returns every directory the engine stages into
/// (behaviour, and `engine_bridge`'s own test that those are the engine's directories).
#[test]
fn a_release_build_sweeps_every_directory_the_engine_stages_into() {
    let candidates = engine_bridge::finder_staging_candidates(crate::APP);
    // macOS: the data root, then the cache root and the temp root that builds before the move used (spec
    // 2026-10-09 §8.4). Elsewhere: the preferred directory and the temp-dir fallback.
    let expected = if cfg!(target_os = "macos") { 3 } else { 2 };
    assert_eq!(
        candidates.len(),
        expected,
        "every directory the engine stages into, now and before"
    );
    assert_eq!(
        release_staging_dirs(),
        candidates,
        "the release list is the engine's complete list, in order"
    );

    let production = production_source();
    let code = code_of(&production, "fn production_staging_dirs(");
    let lines: Vec<&str> = code.lines().map(str::trim).filter(|line| !line.is_empty()).collect();
    assert_eq!(
        lines,
        [
            "fn production_staging_dirs() -> Vec<PathBuf> {",
            "#[cfg(not(test))]",
            "{",
            "release_staging_dirs()",
            "}"
        ],
        "the release arm is one call to release_staging_dirs and nothing else:\n{code}"
    );
    let release = code_of(&production, "fn release_staging_dirs(");
    let release: Vec<&str> = release.lines().map(str::trim).filter(|line| !line.is_empty()).collect();
    assert_eq!(
        release,
        [
            "fn release_staging_dirs() -> Vec<PathBuf> {",
            "engine_bridge::finder_staging_candidates(crate::APP)"
        ],
        "the release list is the engine's own list, not a copy of it"
    );
}

/// F7 (fix round 1 of Task 10): the sign-in token, and the recovery-phrase unlock's token and key, are
/// `Zeroizing`, and the recovery-phrase copies are wiped before the engine start's await instead of living
/// to the end of the function.
#[test]
fn the_sign_in_and_recovery_copies_of_the_token_and_key_are_wiped() {
    let production = production_source();
    for signature in ["async fn desktop_login(", "async fn desktop_login_2fa("] {
        let code = code_of(&production, signature);
        let binding = concat!("let session_", "token");
        assert!(
            code.contains(&format!("{binding} = zeroize::Zeroizing::new(")),
            "{signature}: a plain session token"
        );
        assert_eq!(
            code.matches(&format!("{binding} ")).count(),
            1,
            "{signature} keeps a second plain copy"
        );
    }
    let unlock = code_of(&production, "async fn desktop_unlock_with_recovery_phrase(");
    assert!(
        unlock.contains("let token = zeroize::Zeroizing::new("),
        "the loaded token is wrapped:\n{unlock}"
    );
    assert!(
        !unlock.contains("let master_key = *verified_key;"),
        "no plain copy of the verified key:\n{unlock}"
    );
    let wiped = unlock.find("drop(token);").expect("the token is wiped");
    // The one copy into the session happens in `install_recovered_session` (Task 12), which borrows the key.
    assert!(
        unlock.find("drop(verified_key);").expect("the key is wiped")
            > unlock.find("install_recovered_session(").unwrap()
    );
    let install = code_of(&production, "fn install_recovered_session(");
    assert!(
        install.contains("verified_key: &[u8; 32],") && install.contains("master_key: *verified_key"),
        "{install}"
    );
    let started = unlock.rfind("start_engine_if_possible(").expect("the engine start");
    assert!(
        wiped < started,
        "both copies are gone before the await of the engine start:\n{unlock}"
    );
}

// ---- sign-out and startup (source pins) ----

/// R10: on macOS and Linux a sign-out whose purge fails stops, instead of warning and completing. The shape,
/// as source pins; the behaviour is `the_sign_out_purge_*` below.
#[test]
fn a_failed_sign_out_purge_stops_the_sign_out_on_macos_and_linux() {
    let production = production_source();
    let purge = body_between(&production, "fn purge_local_data_for_sign_out(", "\n}\n");
    assert_eq!(purge.matches("return Err(").count(), 1, "a failed clear stops it");
    assert_eq!(
        purge.matches("SIGN_OUT_PURGE_FAILED").count(),
        3,
        "a failed clear, a failed purge and an unopenable database all stop it"
    );
    assert!(
        purge.contains("db.clear_account_data(owe_finder_removal)"),
        "a sign-out leaves no account row and no owner"
    );
    // Task 1887 (spec 2026-10-10 §7.3): the one warning is the unfinished-marker clear's, a best-effort step after
    // the rows are cleared. A purge or a clear that fails still stops the sign-out.
    assert_eq!(
        purge.matches("tracing::warn!").count(),
        1,
        "no warn-and-continue left but the marker clear's:\n{purge}"
    );
    assert!(
        crate::source_pin::squeeze(purge).contains(&crate::source_pin::squeeze(
            "sign_out::kept_record::clear_unfinished_at(&path)) { tracing::warn!("
        )),
        "the one warning is the marker clear's:\n{purge}"
    );
    let clear = body_between(&production, "async fn clear_session_impl(", "\n}\n");
    assert!(
            crate::source_pin::squeeze(clear).contains(&crate::source_pin::squeeze(
                "purge_local_data_for_sign_out(state_paths::beebeeb_state_dir(), cfg_sync_root.as_deref(), owe_removal, kept.as_ref()).map_err(&kept_with)?;"
            )),
            "sign-out returns the purge's error:\n{clear}"
        );
    assert!(
        clear.contains("let owe_removal = finder_cleanup.is_err();"),
        "a removal the reconciler could not confirm is owed before any engine starts:\n{clear}"
    );
    assert!(
        crate::source_pin::squeeze(clear).contains(&crate::source_pin::squeeze(
            "#[cfg(not(target_os = \"macos\"))] let owe_removal = false;"
        )),
        "off a Mac there is no Finder domain, so a sign-out owes no removal:\n{clear}"
    );
}

/// The upgrade path runs on what the Keychain gave back, before the probe can drop a revoked token and before
/// any engine starts, and the window shuts on the path that returns early too.
#[test]
fn the_startup_restore_adopts_unbound_local_data_before_anything_starts() {
    let production = production_source();
    let restore = body_between(&production, "async fn restore_stored_session(", "\n}\n");
    let load = restore.find("load_session_from_keychain(").expect("the keychain read");
    let adopt = restore
        .find("adopt_unbound_local_data_at_startup(startup_adoption_candidate(&loaded).as_ref());")
        .expect("the upgrade path runs at startup, on what was loaded");
    assert!(load < adopt, "adoption looks at what the Keychain returned");
    assert!(adopt < restore.find("probe_startup_session(").expect("the probe"));
    assert!(
        !restore.contains("start_engine_if_possible("),
        "the engine start comes after, in the caller"
    );
    let inner = body_between(&production, "async fn restore_session_inner(", "\n}\n");
    assert!(
        inner.find("restore_stored_session(").expect("the restore")
            < inner.find("start_engine_if_possible(").expect("the engine start")
    );
    assert!(
        restore.contains("adopt_unbound_local_data_at_startup(None);"),
        "the early return shuts the window too:\n{restore}"
    );
    let candidate = body_between(&production, "fn startup_adoption_candidate(", "\n}\n");
    assert!(
        candidate.contains("loaded.as_ref().ok()?.as_ref()?"),
        "only a session the Keychain returned COMPLETE names a candidate:\n{candidate}"
    );
}

// ---- the upgrade path, with the Keychain the tests use (an in-memory one on a Mac) ----

/// A complete session in the (test) Keychain: token, wrapped vault key and email.
#[cfg(target_os = "macos")]
fn store_complete_session(acct: &crate::account::AccountRuntime, email: &str) {
    persist_session_to_keychain(
        &SessionWrite::for_test(),
        acct.id.as_str(),
        "tok-keychain",
        &[3u8; 32],
        Some(email),
    )
    .expect("the in-memory test store takes it");
}

/// What the startup restore does with the Keychain, for the data in `dir`: read the session, hand the adoption
/// what came back, report who (if anybody) the data now belongs to.
#[cfg(target_os = "macos")]
fn startup_adoption(dir: &Path, acct: &crate::account::AccountRuntime) -> Option<Identity> {
    let loaded = load_session_from_keychain(acct.id.as_str(), None);
    adopt_unbound_local_data_in(dir, startup_adoption_candidate(&loaded).as_ref());
    crate::state_db::StateDb::open(dir.join("state.db"))
        .unwrap()
        .owner()
        .unwrap()
}

/// Lead ruling 8: an install from before the binding keeps its queue. The account whose vault key this
/// computer's Keychain holds is recorded as the owner at startup, and the same account's first engine start
/// then keeps everything.
#[cfg(target_os = "macos")]
#[test]
fn an_upgrader_keeps_the_queue_when_the_keychain_holds_the_same_accounts_vault_key() {
    let fx = AuthorizeFixture::without_session();
    store_complete_session(&fx.acct, "a@beebeeb.io");
    let local = local_data_of(None);
    assert_eq!(
        local.data.db.owner().unwrap(),
        None,
        "the premise: data from before the binding has no owner"
    );
    assert_eq!(
        startup_adoption(local.dir.path(), &fx.acct),
        Some(Identity::new(None, Some("a@beebeeb.io"))),
        "the Keychain's account owns it"
    );

    // The same person signs back in (offline: by the exact email the Keychain names).
    let session = identity_of_session(Some("a@beebeeb.io"), None);
    assert_eq!(bind_data_to_session(&fx.state, &local.data, &session), Ok(()));
    assert_eq!(
        local.data.db.list_due_operations(i64::MAX).unwrap().len(),
        1,
        "the queue survived the upgrade"
    );
    for path in &local.files {
        assert!(path.exists(), "{} survived the upgrade", path.display());
    }
}

/// The same upgrade, but another account starts: it does not inherit the queue.
#[cfg(target_os = "macos")]
#[test]
fn an_upgrade_never_hands_the_queue_to_another_account() {
    let fx = AuthorizeFixture::without_session();
    store_complete_session(&fx.acct, "a@beebeeb.io");
    let local = local_data_of(None);
    startup_adoption(local.dir.path(), &fx.acct);
    let bob_session = Identity::new(None, Some("b@beebeeb.io"));
    assert_eq!(bind_data_to_session(&fx.state, &local.data, &bob_session), Ok(()));
    assert!(
        local.data.db.list_due_operations(i64::MAX).unwrap().is_empty(),
        "another account does not inherit the queue"
    );
}

// ---- the Keychain keeps an email only next to the session it names (fix round 4) ----

/// A sign-in that has no identity (its account-record fetch failed: `apply_session` stores `session_email(None)`)
/// stores its token and vault key with NO email. The email the Keychain kept for the account signed in before is
/// removed, so the stored session is unidentified and is never taken for that account.
#[cfg(target_os = "macos")]
#[test]
fn an_unidentified_sign_in_removes_the_previous_accounts_email_from_the_keychain() {
    let fx = AuthorizeFixture::without_session();
    let id = fx.acct.id.as_str().to_string();
    store_complete_session(&fx.acct, "a@beebeeb.io");
    assert_eq!(
        keychain_account_email(&id).as_deref(),
        Some("a@beebeeb.io"),
        "the premise: the Keychain names the account before"
    );
    persist_session_to_keychain(
        &SessionWrite::for_test(),
        &id,
        "tok-b",
        &[5u8; 32],
        session_email(None).as_deref(),
    )
    .expect("the in-memory test store takes it");
    assert_eq!(
        keychain_account_email(&id),
        None,
        "no email is kept next to the new token and key"
    );
    let restored = load_session_from_keychain(&id, None)
        .unwrap()
        .expect("the new session is stored");
    assert_eq!(
        (restored.token.as_str(), restored.master_key, restored.email.clone()),
        ("tok-b", [5u8; 32], None),
        "restored without an identity"
    );
}

/// What a relaunch then does, step by step as `restore_session_inner` runs it (the restore itself needs an
/// `AppHandle`): read the Keychain (no email in memory yet), offer the adoption what it read, install the session,
/// try to name it (the server cannot be reached), start the engine. The session is unidentified: no engine, nothing
/// purged, nobody adopted, no owner replaced, no Finder debt. For the previous account's data and for unowned data.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn after_an_unidentified_sign_in_a_relaunch_starts_nothing_purges_nothing_and_adopts_nothing() {
    for (name, owner) in [
        ("the previous account's data", Some(alice())),
        ("data with no recorded owner", None),
    ] {
        let fx = AuthorizeFixture::without_session();
        let id = fx.acct.id.as_str().to_string();
        store_complete_session(&fx.acct, "a@beebeeb.io");
        let local = local_data_of(owner.clone());
        persist_session_to_keychain(
            &SessionWrite::for_test(),
            &id,
            "tok-b",
            &[5u8; 32],
            session_email(None).as_deref(),
        )
        .expect("the in-memory test store takes it");

        let loaded = load_session_from_keychain(&id, None);
        adopt_unbound_local_data_in(local.dir.path(), startup_adoption_candidate(&loaded).as_ref());
        let owner_after_adoption = local.data.db.owner().unwrap();
        let session = loaded.unwrap().expect("the new session is stored");
        let restored_email = session.email.clone();
        *fx.acct.session.lock().unwrap() = Some(session);
        establish_session_identity(
            &fx.state,
            &fx.acct,
            &answering("503 Service Unavailable", "{}".to_string()),
            None,
        )
        .await;
        let mut slot = fx.acct.engine.try_lock().unwrap();
        let paths = LocalDataPaths::for_test(local.dir.path(), &local.staging);
        let seen = std::cell::RefCell::new(None);
        let result = start_engine_bound(&fx.state, &fx.acct, &mut slot, &paths, |_root, _token, _key| {
            *seen.borrow_mut() = Some((
                local.data.db.list_due_operations(i64::MAX).unwrap().len(),
                local.files.iter().filter(|path| path.exists()).count(),
            ));
            crate::runner::EngineRunner::for_test_with_task(tokio::spawn(async {}))
        });
        assert_eq!(
            (owner_after_adoption, restored_email, result, seen.into_inner()),
            (
                owner.clone(),
                None,
                Err(crate::account_binding::IDENTITY_UNKNOWN.to_string()),
                None
            ),
            "{name}: (owner after the adoption, restored email, engine start, what an engine was created with)"
        );
        assert!(slot.is_none(), "{name}: no engine in the slot");
        assert_eq!(
            local.data.db.list_due_operations(i64::MAX).unwrap().len(),
            1,
            "{name}: nothing purged"
        );
        for path in &local.files {
            assert!(path.exists(), "{name}: {} is kept", path.display());
        }
        assert_eq!(local.data.db.owner().unwrap(), owner, "{name}: the owner is unchanged");
        assert!(!local.data.db.finder_removal_owed().unwrap(), "{name}: no Finder debt");
    }
}

/// An identified sign-in stores its OWN email in place of the previous account's: the browser sign-in (token and
/// key) and the password and 2FA sign-ins (token only). A token-only write without an email (no sign-in does that
/// today; the writer is shared) leaves none either.
#[cfg(target_os = "macos")]
#[test]
fn an_identified_sign_in_stores_its_own_email_in_place_of_the_previous_accounts() {
    let email = session_email(Some(&profile_of("u-b", "b@beebeeb.io")));
    for (name, token_only) in [("browser", false), ("password or 2FA", true)] {
        let fx = AuthorizeFixture::without_session();
        let id = fx.acct.id.as_str().to_string();
        store_complete_session(&fx.acct, "a@beebeeb.io");
        if token_only {
            persist_session_token_to_keychain(&SessionWrite::for_test(), &id, "tok-b", email.as_deref()).unwrap();
        } else {
            persist_session_to_keychain(&SessionWrite::for_test(), &id, "tok-b", &[5u8; 32], email.as_deref()).unwrap();
            let restored = load_session_from_keychain(&id, None).unwrap().expect("stored");
            assert_eq!(
                restored.email.as_deref(),
                Some("b@beebeeb.io"),
                "{name}: the restore knows the session by its own email"
            );
        }
        assert_eq!(keychain_account_email(&id).as_deref(), Some("b@beebeeb.io"), "{name}");
    }
    let fx = AuthorizeFixture::without_session();
    let id = fx.acct.id.as_str().to_string();
    store_complete_session(&fx.acct, "a@beebeeb.io");
    persist_session_token_to_keychain(&SessionWrite::for_test(), &id, "tok-c", None).unwrap();
    assert_eq!(
        keychain_account_email(&id),
        None,
        "a token-only write without an email keeps none"
    );
}

/// Every write of a session token removes the email kept for the session before it, BEFORE the token is stored
/// (so no failure part-way can leave the new token next to the old email), and a removal that fails stops the
/// write; the session's own email is written after its secrets. The vault key alone is written only by the
/// recovery-phrase unlock, for the token already stored, whose email was written or removed with that token.
#[test]
fn every_write_of_a_session_token_removes_the_previous_email_first() {
    let production = production_source();
    assert_eq!(
        production.matches(".install_session(").count(),
        2,
        "the two writers of a session token"
    );
    for signature in [
        "fn persist_session_to_keychain(",
        "fn persist_session_token_to_keychain(",
    ] {
        let code = code_of(&production, signature);
        let forget = code
            .find("forget_account_email_in_keychain(account_id)?;")
            .unwrap_or_else(|| {
                panic!("{signature}: removes the previous email, and a failure stops the write:\n{code}")
            });
        let token = code.find(".install_session(").expect("the token");
        assert!(
            forget < token,
            "{signature}: the email goes before the token is stored:\n{code}"
        );
        let own = code
            .find("persist_account_email_to_keychain(&vault, email)")
            .unwrap_or_else(|| panic!("{signature}: its own email"));
        assert!(
            token < own,
            "{signature}: the session's own email is written after its secrets:\n{code}"
        );
    }
    let forget = code_of(&production, "fn forget_account_email_in_keychain(");
    assert!(
        forget.contains(".delete_account_email()") && !forget.contains("let _"),
        "{forget}"
    );
    assert_eq!(
        production.matches(".store_wrapped_master_key(").count(),
        2,
        "with a new token, and for the recovery phrase"
    );
    assert_eq!(
        production.matches("persist_vault_key_to_keychain(").count(),
        2,
        "defined once, called by the recovery-phrase unlock only"
    );
    let recovery = code_of(&production, "async fn desktop_unlock_with_recovery_phrase(");
    assert!(
        recovery.contains("load_session_token_from_keychain(&account_id)?"),
        "the key is for the token already stored"
    );
}

/// F2, across restarts: unowned data from before the upgrade, and a Keychain that holds a token but no vault key.
/// The startup pass adopts nobody and uses up the one window; once the key is added and the session installed,
/// the engine starts only on a clean slate; at every later startup the Keychain is complete and adopts nothing.
#[cfg(target_os = "macos")]
#[test]
fn adoption_needs_a_vault_key_and_the_window_closes_at_the_first_startup() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let fx = AuthorizeFixture::without_session();
        let id = fx.acct.id.as_str().to_string();
        let local = local_data_of(None); // another account's queue and files, no owner
        // 1. A sign-in stores a token and an email, and no key yet.
        persist_session_token_to_keychain(&SessionWrite::for_test(), &id, "tok-b", Some("b@beebeeb.io")).unwrap();
        // 2. A startup: the Keychain holds no vault key for that token, so nobody is adopted.
        let loaded = load_session_from_keychain(&id, None);
        assert!(loaded.is_err(), "a token without a key does not restore: {loaded:?}");
        assert_eq!(startup_adoption_candidate(&loaded), None);
        assert_eq!(startup_adoption(local.dir.path(), &fx.acct), None, "nobody was adopted");
        // 3. The recovery phrase is entered: the key is stored and the session is installed.
        persist_vault_key_to_keychain(&SessionWrite::for_test(), &id, &[3u8; 32]).unwrap();
        *fx.acct.session.lock().unwrap() = Some(Session {
            token: "tok-b".into(),
            master_key: [3u8; 32],
            email: Some("b@beebeeb.io".into()),
        });
        // 4. B's engine starts. When it is created the previous account's queue and files are gone. (The Mac
        //    first waits for the previous account's Finder domain to be removed: a confirmed removal frees it.)
        let mut slot = fx.acct.engine.lock().await;
        let seen = std::cell::RefCell::new(None);
        let paths = LocalDataPaths::for_test(local.dir.path(), &local.staging);
        let spawner = || {
            |_root: PathBuf, _token: zeroize::Zeroizing<String>, _key: zeroize::Zeroizing<[u8; 32]>| {
                *seen.borrow_mut() = Some((
                    local.data.db.list_due_operations(i64::MAX).unwrap().len(),
                    local.files.iter().filter(|path| path.exists()).count(),
                ));
                crate::runner::EngineRunner::for_test_with_task(tokio::spawn(async {}))
            }
        };
        let first = start_engine_bound(&fx.state, &fx.acct, &mut slot, &paths, spawner());
        assert_eq!(
            first,
            Ok(EngineStart::FinderRemovalOwed),
            "the reset leaves the previous account's domain to remove first"
        );
        assert!(seen.borrow().is_none(), "no engine while the removal is owed");
        clear_finder_removal_owed_in(local.dir.path());
        let started = start_engine_bound(&fx.state, &fx.acct, &mut slot, &paths, spawner());
        assert_eq!(started, Ok(EngineStart::Started));
        assert_eq!(
            seen.into_inner(),
            Some((0, 0)),
            "the engine saw none of the previous account's data"
        );
        drop(slot);
        // 5. A later startup: the Keychain is complete and names the signed-in account. The window is shut, so
        //    nothing is adopted, and nothing of the previous account is back.
        let loaded = load_session_from_keychain(&id, None);
        assert_eq!(
            startup_adoption_candidate(&loaded),
            Some(Identity::new(None, Some("b@beebeeb.io")))
        );
        assert_eq!(
            startup_adoption(local.dir.path(), &fx.acct),
            Some(Identity::new(None, Some("b@beebeeb.io")))
        );
        assert!(local.data.db.list_due_operations(i64::MAX).unwrap().is_empty());
    });
}

/// And the window really is shut: with a complete Keychain at a LATER startup, unowned data is not adopted.
#[cfg(target_os = "macos")]
#[test]
fn a_later_startup_never_adopts_even_with_a_complete_keychain() {
    let fx = AuthorizeFixture::without_session();
    let id = fx.acct.id.as_str().to_string();
    let local = local_data_of(None);
    persist_session_token_to_keychain(&SessionWrite::for_test(), &id, "tok-b", Some("b@beebeeb.io")).unwrap();
    assert_eq!(
        startup_adoption(local.dir.path(), &fx.acct),
        None,
        "the first startup: no vault key"
    );
    persist_vault_key_to_keychain(&SessionWrite::for_test(), &id, &[3u8; 32]).unwrap();
    assert_eq!(
        startup_adoption(local.dir.path(), &fx.acct),
        None,
        "a later startup with a complete Keychain: too late"
    );
    assert_eq!(local.data.db.owner().unwrap(), None);
}
