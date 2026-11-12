use super::finder_setup_command_tests::{body_between, production_source};
use super::upload_staging_dir_for_session_purge;
use super::{AppState, clear_session_impl, set_auth_email, set_auth_present};
// Only the Unix tests (upload staging is a macOS folder; Linux tests drive the same code) use these.
#[cfg(unix)]
use super::{stop_engine_for_lock, stop_engine_for_sign_out};
// Only the macOS/Linux assertion in the already-signed-out test names it; on Windows the import would be unused.
#[cfg(not(target_os = "windows"))]
use super::UNCONFIRMED_ENGINE_STOP_ERROR;
use crate::account::{AccountId, synthesize_single_account};
use crate::runner::EngineRunner;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

fn test_account(state: &AppState, id: &str) -> Arc<crate::account::AccountRuntime> {
    synthesize_single_account(state, AccountId(id.to_string()));
    state.active_account().expect("synthesized account resolves")
}

// ── Upload staging at sign-out and Lock ─────────────────────────────────
// A throwaway staging dir in every test: never the real App Group
// container (`upload_staging_dir_for_session_purge` is None under test,
// unless the test injects its own folder with `with_upload_staging_for_test`).

/// A staging dir holding a handed-over copy and a stray tree, next to a
/// file the purge must never reach.
#[cfg(unix)]
fn staging_with_copies() -> (tempfile::TempDir, std::path::PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let staging = root.path().join("upload-staging");
    std::fs::create_dir(&staging).unwrap();
    std::fs::write(staging.join("copy"), b"plaintext").unwrap();
    std::fs::create_dir(staging.join("stray-tree")).unwrap();
    std::fs::write(staging.join("stray-tree").join("inside"), b"plaintext").unwrap();
    std::fs::write(root.path().join("outside.txt"), b"never purged").unwrap();
    (root, staging)
}

#[cfg(unix)]
fn staging_entries(staging: &std::path::Path) -> usize {
    std::fs::read_dir(staging).unwrap().count()
}

#[cfg(unix)]
fn assert_outside_untouched(root: &tempfile::TempDir) {
    assert_eq!(std::fs::read(root.path().join("outside.txt")).unwrap(), b"never purged");
}

/// An engine whose task already finished: its stop is confirmed at once.
#[cfg(unix)]
fn finished_engine() -> EngineRunner {
    EngineRunner::for_test_with_task(tokio::spawn(async {}))
}

/// An engine whose task has no await point, so even the forced abort
/// cannot confirm it stopped (3 s graceful + 2 s forced). It ends by
/// itself after 7 s, so it cannot leak a spinning thread.
#[cfg(unix)]
fn unstoppable_engine() -> EngineRunner {
    let deadline = std::time::Instant::now() + Duration::from_secs(7);
    EngineRunner::for_test_with_task(tokio::task::spawn_blocking(move || {
        while std::time::Instant::now() < deadline {
            std::hint::black_box(());
        }
    }))
}

#[test]
fn the_session_purge_never_targets_the_real_app_group_directory_under_test() {
    assert_eq!(upload_staging_dir_for_session_purge(), None);
}

#[cfg(unix)]
#[tokio::test]
async fn sign_out_after_a_confirmed_engine_stop_empties_upload_staging() {
    let state = AppState::default();
    let acct = test_account(&state, "signout-staging-confirmed");
    let (root, staging) = staging_with_copies();
    let mut slot = Some(finished_engine());
    stop_engine_for_sign_out(&acct, &mut slot, Some(&staging))
        .await
        .expect("a confirmed stop lets sign-out continue");
    assert!(slot.is_none(), "the engine is consumed");
    assert_eq!(staging_entries(&staging), 0, "sign-out removes every handed-over copy");
    assert!(staging.is_dir(), "the directory itself stays");
    assert_outside_untouched(&root);
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sign_out_with_an_unconfirmed_engine_stop_purges_no_upload_staging() {
    let state = AppState::default();
    let acct = test_account(&state, "signout-staging-unconfirmed");
    let (root, staging) = staging_with_copies();
    let mut slot = Some(unstoppable_engine());
    let error = stop_engine_for_sign_out(&acct, &mut slot, Some(&staging))
        .await
        .expect_err("an unconfirmed stop refuses sign-out");
    assert!(error.contains("Could not stop the sync engine"), "got: {error}");
    assert_eq!(
        staging_entries(&staging),
        2,
        "nothing is purged while the engine may still be serving requests"
    );
    // The retry finds an empty slot; the unconfirmed-stop flag still refuses it.
    let mut empty = None;
    stop_engine_for_sign_out(&acct, &mut empty, Some(&staging))
        .await
        .expect_err("the retry is refused too");
    assert_eq!(staging_entries(&staging), 2, "and still purges nothing");
    assert_outside_untouched(&root);
}

/// The sign-out's real path (review of the merge, I1, and spec §8.4): `clear_session_impl` stops the engine
/// through `take_slot_and_stop_engine_for_sign_out`, which empties the upload-staging folder, and its purge
/// removes the staged Finder-write copies in every staging root the allow-list must accept: the data root
/// macOS stages in now and the cache root that earlier builds used. Driven on the already-signed-out path,
/// with a real upload-staging folder and the copies' journal rows in the shared scratch state database.
#[cfg(unix)]
#[tokio::test]
async fn a_sign_out_empties_upload_staging_and_the_staged_copies_in_every_staging_root() {
    let state_dir = crate::state_paths::init_for_test();
    let state = AppState::default();
    let acct = test_account(&state, "signout-staging-roots");
    #[cfg(target_os = "macos")]
    {
        let (handle, rx) = crate::finder_setup::driver::FinderSetupHandle::for_test(
            crate::finder_setup::driver::FinderSetupView::initial(
                crate::finder_setup::launch_location::LaunchLocation::Applications,
            ),
        );
        let _ = state.finder_setup.set(handle);
        tokio::spawn(super::confirm_every_finder_event(rx));
    }
    set_auth_present(&state, false);
    *acct.engine.lock().await = Some(finished_engine());
    let (root, upload_staging) = staging_with_copies();
    // The roots of the test sandbox (never the person's real folders, never the shared temp dir).
    let bases = crate::engine_bridge::FinderStagingBases::current(crate::APP);
    #[cfg(target_os = "macos")]
    let staging_roots = [
        bases.durable_root().expect("the test sandbox has a data dir"),
        bases.cache_root(),
    ];
    #[cfg(not(target_os = "macos"))]
    let staging_roots = [bases.cache_root()];
    let copies: Vec<std::path::PathBuf> = staging_roots
        .iter()
        .map(|staging_root| {
            std::fs::create_dir_all(staging_root).unwrap();
            let copy = staging_root.join(uuid::Uuid::new_v4().to_string());
            std::fs::write(&copy, b"plaintext").unwrap();
            copy
        })
        .collect();

    let result = super::with_the_shared_state_dir(async {
        let db = crate::state_db::StateDb::open(state_dir.join(crate::state_paths::STATE_DB_FILENAME)).unwrap();
        for copy in &copies {
            db.track_staged_payload(&copy.to_string_lossy(), Some("/source"), false)
                .unwrap();
        }
        drop(db);
        super::with_upload_staging_for_test(
            upload_staging.clone(),
            clear_session_impl(&state, false, None, crate::UnknownFilesArg::Keep, None),
        )
        .await
    })
    .await;

    assert!(result.is_ok(), "the sign-out completes: {result:?}");
    assert_eq!(
        staging_entries(&upload_staging),
        0,
        "the sign-out's engine stop empties upload staging"
    );
    for copy in &copies {
        assert!(!copy.exists(), "the staged copy is purged: {copy:?}");
    }
    assert_outside_untouched(&root);
}

#[cfg(unix)]
#[tokio::test]
async fn lock_after_a_confirmed_engine_stop_empties_upload_staging() {
    let state = AppState::default();
    let acct = test_account(&state, "lock-staging-confirmed");
    let (root, staging) = staging_with_copies();
    let mut slot = Some(finished_engine());
    stop_engine_for_lock(&acct, &mut slot, Some(&staging))
        .await
        .expect("the lock continues");
    assert!(slot.is_none());
    assert_eq!(staging_entries(&staging), 0, "Lock removes every handed-over copy");
    assert_outside_untouched(&root);
}

#[cfg(unix)]
#[tokio::test]
async fn lock_with_no_engine_running_empties_upload_staging() {
    // No engine, so no IPC listener: nothing can still be using a copy.
    let state = AppState::default();
    let acct = test_account(&state, "lock-staging-no-engine");
    let (root, staging) = staging_with_copies();
    let mut slot = None;
    stop_engine_for_lock(&acct, &mut slot, Some(&staging))
        .await
        .expect("the lock continues");
    assert_eq!(staging_entries(&staging), 0);
    assert_outside_untouched(&root);
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lock_with_an_unconfirmed_engine_stop_does_not_purge_upload_staging() {
    let state = AppState::default();
    let acct = test_account(&state, "lock-staging-unconfirmed");
    let (root, staging) = staging_with_copies();
    let mut slot = Some(unstoppable_engine());
    stop_engine_for_lock(&acct, &mut slot, Some(&staging))
        .await
        .expect("off Windows the lock goes ahead on an unconfirmed stop");
    assert_eq!(
        staging_entries(&staging),
        2,
        "an IPC listener that may still be serving could be handed a copy the purge just removed"
    );
    assert_outside_untouched(&root);
}

/// #139 review P2-3 (task 1887's rule, nothing of the account survives sign-out): the storage summary is gone after
/// a sign-out, so a used/quota figure of the account before is not kept in memory.
#[tokio::test]
async fn sign_out_clears_the_storage_summary() {
    crate::state_paths::init_for_test();
    let state = AppState::default();
    let _acct = test_account(&state, "signout-storage-acct");
    #[cfg(target_os = "macos")]
    {
        let (handle, rx) = crate::finder_setup::driver::FinderSetupHandle::for_test(
            crate::finder_setup::driver::FinderSetupView::initial(
                crate::finder_setup::launch_location::LaunchLocation::Applications,
            ),
        );
        let _ = state.finder_setup.set(handle);
        tokio::spawn(super::confirm_every_finder_event(rx));
    }
    let seeded = crate::popover_data::cached_storage(&state.popover_storage, 1_000, 7, || async {
        Ok(crate::popover_data::StorageUsage {
            used_bytes: 84,
            quota_bytes: 200,
        })
    })
    .await;
    assert_eq!(seeded.map(|s| s.used_bytes), Some(84));
    assert!(!state.popover_storage.lock().unwrap().is_empty());
    set_auth_present(&state, false);
    let result = clear_session_impl(&state, false, None, crate::UnknownFilesArg::Keep, None).await;
    assert!(result.is_ok(), "{result:?}");
    assert!(
        state.popover_storage.lock().unwrap().is_empty(),
        "{:?}",
        state.popover_storage.lock().unwrap()
    );
}

/// Bug B: sign-out while already signed out (no auth flag, no in-memory
/// session, no keychain session) must SUCCEED as a no-op — it must not
/// attempt the engine stop (a live engine in the slot must still be
/// running afterwards), must not consume the engine slot, and must not
/// fail on the (Linux-stub) keychain clear.
///
/// Superseded on macOS and Linux by Task 12 fix round 2, item 1 (lead ruling): the already-signed-out path takes
/// the slot before its purge and STOPS an engine it finds there (one a sign-in that completed during the
/// sign-out started), so no engine runs on after `Ok`. It still succeeds. Windows keeps the Bug B behaviour
/// asserted here.
#[tokio::test]
async fn clear_session_when_already_signed_out_skips_the_engine_and_succeeds() {
    // Every platform runs the local-state purge on this path, and a purge that cannot run stops
    // the sign-out (R10), so it needs the process-wide state dir the real app sets at startup.
    crate::state_paths::init_for_test();
    let state = AppState::default();
    let acct = test_account(&state, "signout-noop-acct");
    // A Mac's sign-out waits for the reconciler's removal and reports one it cannot confirm.
    #[cfg(target_os = "macos")]
    {
        let (handle, rx) = crate::finder_setup::driver::FinderSetupHandle::for_test(
            crate::finder_setup::driver::FinderSetupView::initial(
                crate::finder_setup::launch_location::LaunchLocation::Applications,
            ),
        );
        let _ = state.finder_setup.set(handle);
        tokio::spawn(super::confirm_every_finder_event(rx));
    }
    set_auth_present(&state, false);
    assert!(acct.session.lock().unwrap().is_none());

    // A live engine in the slot: if teardown tried to stop it, the tick
    // counter freezes — the liveness assertion below catches that.
    let ticks = Arc::new(AtomicUsize::new(0));
    let ticks_for_task = ticks.clone();
    let task = tokio::spawn(async move {
        loop {
            ticks_for_task.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    });
    *acct.engine.lock().await = Some(EngineRunner::for_test_with_task(task));

    let result = clear_session_impl(&state, false, None, crate::UnknownFilesArg::Keep, None).await;

    assert!(
        result.is_ok(),
        "signing out while already signed out must succeed, got {result:?}"
    );
    #[cfg(not(target_os = "windows"))]
    {
        assert!(
            acct.engine.lock().await.is_none(),
            "macOS/Linux: the engine it found is stopped (fix round 2, item 1)"
        );
        let after = ticks.load(Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(120)).await;
        assert_eq!(ticks.load(Ordering::SeqCst), after, "and it no longer runs");
    }
    #[cfg(target_os = "windows")]
    {
        assert!(
            acct.engine.lock().await.is_some(),
            "the no-op path must not consume the engine slot"
        );
        let before = ticks.load(Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(120)).await;
        assert!(
            ticks.load(Ordering::SeqCst) > before,
            "the engine task must still be running — the already-signed-out path \
                 must not attempt an engine abort"
        );
    }
}

/// Fix round 3, M2 (re-review): an already-signed-out sign-out after an engine stop that was never confirmed. On
/// macOS and Linux this path takes the engine slot through the one helper, so it refuses with the same sentence as
/// the signed-in path (the stop it would need cannot be trusted; a restart clears the flag). Windows keeps its
/// already-signed-out path, which takes no slot and succeeds.
#[tokio::test]
async fn an_already_signed_out_sign_out_after_an_unconfirmed_engine_stop_refuses_on_macos_and_linux() {
    crate::state_paths::init_for_test();
    let state = AppState::default();
    let acct = test_account(&state, "signout-unconfirmed-acct");
    #[cfg(target_os = "macos")]
    {
        let (handle, rx) = crate::finder_setup::driver::FinderSetupHandle::for_test(
            crate::finder_setup::driver::FinderSetupView::initial(
                crate::finder_setup::launch_location::LaunchLocation::Applications,
            ),
        );
        let _ = state.finder_setup.set(handle);
        tokio::spawn(super::confirm_every_finder_event(rx));
    }
    set_auth_present(&state, false);
    assert!(
        acct.session.lock().unwrap().is_none(),
        "the premise: already signed out"
    );
    acct.engine_stop_unconfirmed.store(true, Ordering::SeqCst);
    let result = clear_session_impl(&state, false, None, crate::UnknownFilesArg::Keep, None).await;
    #[cfg(not(target_os = "windows"))]
    assert_eq!(
        result,
        Err(UNCONFIRMED_ENGINE_STOP_ERROR.into()),
        "macOS/Linux: refused"
    );
    #[cfg(target_os = "windows")]
    assert!(result.is_ok(), "Windows: unchanged, {result:?}");
    assert!(
        acct.engine_stop_unconfirmed.load(Ordering::SeqCst),
        "the flag stays until a restart"
    );
}

/// Fix round 3, M3: the already-signed-out path's log line and comments say what each platform does now.
#[test]
fn the_already_signed_out_sign_out_says_what_each_platform_does() {
    let production = production_source();
    let clear = body_between(&production, "async fn clear_session_impl(", "\n}\n");
    let windows = clear
            .find("#[cfg(target_os = \"windows\")]\n        tracing::info!(\"sign-out requested while already signed out; skipping engine stop and Cloud Files teardown\");")
            .expect("Windows says it skips the engine stop");
    let elsewhere = clear
            .find("#[cfg(not(target_os = \"windows\"))]\n        tracing::info!(\"sign-out requested while already signed out; stopping any engine still in the slot\");")
            .expect("macOS and Linux say they stop any engine");
    assert!(windows < elsewhere, "{clear}");
    assert_eq!(
        clear.matches("skipping engine stop").count(),
        1,
        "only Windows says it skips the stop:\n{clear}"
    );
    assert!(
        !clear.contains("stopped nothing and holds none, so it takes the slot here"),
        "the stale comment is gone"
    );
    assert!(
        clear.contains("except the Windows\n    // already-signed-out path, which stopped nothing and holds none"),
        "{clear}"
    );
    assert!(
        !clear.contains("→ skip the engine stop and the Cloud"),
        "the Bug B comment no longer says every platform skips the stop"
    );
}

/// Task 1886 (P0): the sign-out ends on the server every session it took, through ONE call that every OS compiles
/// (it was Windows only). Pinned as text so it holds on Windows too, where no test can drive a signed-in sign-out:
/// no `#[cfg(` gates the first token read or the call (looked for past every comment and blank line above them,
/// review finding 4); the first token is read before the engine slot is taken (from the slot on nothing touches the
/// session until its clear); every clear hands what it removes to the same collector; and the call comes after the
/// last local clear step and after every release of the engine slot (review findings 1 to 3).
#[test]
fn the_sign_out_ends_its_session_on_the_server_on_every_os() {
    let production = production_source();
    let clear = body_between(&production, "async fn clear_session_impl(", "\n}\n");
    // The first line above `at` that is code or an attribute: a `#[cfg(` there gates the statement at `at`, however
    // many comment lines come between.
    let not_gated = |at: usize, what: &str| {
        let above = clear[..at]
            .lines()
            .rev()
            .map(str::trim)
            .find(|line| !line.is_empty() && !line.starts_with("//"))
            .unwrap_or_default();
        assert!(!above.starts_with("#[cfg"), "{what} is gated to one platform: {above}");
    };
    let find = |needle: &str| clear.find(needle).unwrap_or_else(|| panic!("{needle}:\n{clear}"));
    let collector = find("    let mut dropped = DroppedTokens::default();\n");
    not_gated(collector, "the collector");
    let first = find("    if !already_signed_out {\n        dropped.add(session_token_for_revoke(&acct));\n    }\n");
    not_gated(first, "the first token read");
    assert!(
        first < find("take_slot_and_stop_engine_for_sign_out("),
        "read before the engine slot is taken"
    );
    let call = find("    end_dropped_sessions(&acct, dropped, cleared.is_ok()).await;\n");
    not_gated(call, "the call");
    for (needle, count) in [
        ("session_token_for_revoke(", 1),
        ("end_dropped_sessions(", 1),
        ("revoke_sessions_for_sign_out(", 0),
        ("revoke_desktop_session(", 0),
        ("dropped.add(in_memory);", 1),
        ("finish_sign_out_turn(&acct, &engine_slot, true, &mut dropped)", 1),
        ("finish_sign_out_turn(&acct, &engine_slot, false, &mut dropped)", 1),
        ("clear_keychain_session_into(acct.id.as_str(), &mut dropped)", 2),
        ("clear_keychain_session(", 0),
    ] {
        assert_eq!(clear.matches(needle).count(), count, "{needle}:\n{clear}");
    }
    assert!(
        find("let in_memory =\n            clear_session_holding_slot(") < find("dropped.add(in_memory);"),
        "the memory clear's token is collected"
    );
    for last_local_step in [
        "drop(engine_slot)",
        "finish_sign_out_turn(",
        "clear_keychain_session_into(",
        "set_auth_present(state, false);",
    ] {
        let at = clear
            .rfind(last_local_step)
            .unwrap_or_else(|| panic!("{last_local_step}"));
        assert!(at < call, "the sessions are ended after {last_local_step}");
    }
    let ended = body_between(&production, "async fn end_dropped_sessions(", "\n}\n");
    assert!(
        ended.contains("session_tokens_held_here(acct)"),
        "a failed clear ends only what is gone:\n{ended}"
    );
    assert!(
        ended.contains("revoke_sessions_for_sign_out(&tokens, &sign_out_api_base())"),
        "{ended}"
    );
}

/// User amendment: the email to prefill into the sign-in form must be
/// captured at sign-out time, BEFORE `clear_keychain_session` erases the
/// keychain account-email credential. Precedence: in-memory session
/// email → mirrored `auth_email` → keychain credential (None on the
/// Linux fail-closed stub, which is also what a genuinely-absent
/// credential yields).
#[tokio::test]
async fn signout_email_to_preserve_prefers_the_session_email() {
    let state = AppState::default();
    let acct = test_account(&state, "signout-email-acct");
    let session = super::Session {
        token: "tok".into(),
        master_key: [7u8; 32],
        email: Some("session@example.com".into()),
    };
    *acct.session.lock().unwrap() = Some(session);
    set_auth_email(&state, Some("mirror@example.com".into()));

    assert_eq!(
        super::signout_email_to_preserve(&acct).as_deref(),
        Some("session@example.com"),
        "the in-memory session email is the most authoritative source"
    );
}

#[tokio::test]
async fn signout_email_to_preserve_falls_back_to_the_mirror_then_none() {
    let state = AppState::default();
    let acct = test_account(&state, "signout-email-acct-2");
    set_auth_email(&state, Some("mirror@example.com".into()));
    assert_eq!(
        super::signout_email_to_preserve(&acct).as_deref(),
        Some("mirror@example.com"),
        "without a session, the mirrored auth_email is captured"
    );

    // Both in-memory sources absent → the keychain credential decides.
    // On the Linux stub store (and for a synthetic account id with no
    // stored credential anywhere) that is None — nothing to prefill.
    set_auth_email(&state, None);
    assert_eq!(
        super::signout_email_to_preserve(&acct),
        None,
        "no session, no mirror, no keychain credential → nothing to prefill"
    );
}

#[test]
fn stage_last_signed_in_email_skips_none_and_unchanged_values() {
    let mut cfg = crate::config::DesktopConfig::default();

    // Nothing signed in → no change, no save.
    assert!(!super::stage_last_signed_in_email(&mut cfg, None));
    assert_eq!(cfg.last_signed_in_email, None);

    // First email → change (caller saves).
    assert!(super::stage_last_signed_in_email(&mut cfg, Some("user@example.com")));
    assert_eq!(cfg.last_signed_in_email.as_deref(), Some("user@example.com"));

    // Same email again → no change, so sign-out never rewrites the
    // desktop.toml just for the prefill.
    assert!(!super::stage_last_signed_in_email(&mut cfg, Some("user@example.com")));

    // A different account's email → change.
    assert!(super::stage_last_signed_in_email(&mut cfg, Some("other@example.com")));
    assert_eq!(cfg.last_signed_in_email.as_deref(), Some("other@example.com"));
}

/// Bug A2: after an abort that cannot CONFIRM the engine stopped, the
/// consumed handle leaves an empty engine slot. A retry must NOT sail
/// past the stop gate (and into the purge) just because the slot is
/// empty — it must be refused with the same unconfirmed-stop error until
/// the process restarts.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn clear_session_retry_after_unconfirmed_engine_stop_cannot_skip_the_stop_gate() {
    let state = AppState::default();
    let acct = test_account(&state, "signout-retry-acct");
    // Keep the already-signed-out guard out of the way: a failed sign-out
    // attempt leaves the auth flag up.
    set_auth_present(&state, true);

    // A task with NO await point: tokio cannot force-abort it, so the
    // forced path times out unconfirmed — the Bug A precondition. It
    // exits on its own after a wall-clock deadline so it cannot leak a
    // spinning worker into the rest of the test run.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let task = tokio::task::spawn_blocking(move || {
        while std::time::Instant::now() < deadline {
            std::hint::black_box(());
        }
    });
    *acct.engine.lock().await = Some(EngineRunner::for_test_with_task(task));

    let first = clear_session_impl(&state, false, None, crate::UnknownFilesArg::Keep, None)
        .await
        .expect_err("the first attempt must fail while the engine stop is unconfirmed");
    assert!(
        first.message.contains("Could not stop the sync engine"),
        "first attempt must be the unconfirmed-stop refusal, got: {first}"
    );

    let second = clear_session_impl(&state, false, None, crate::UnknownFilesArg::Keep, None)
        .await
        .expect_err("the retry must also be refused, not silently proceed past the stop gate");
    assert!(
        second.message.contains("Could not stop the sync engine"),
        "the retry must be gated by the same unconfirmed-stop refusal \
             (Bug A2: it must not skip the stop gate on an empty slot), got: {second}"
    );
}
