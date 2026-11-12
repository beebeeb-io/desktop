use super::loopback_api_for_tests::LoopbackApi;
use super::*;
use std::sync::atomic::Ordering;
use tauri::Manager;

/// The command reads `BB_API_BASE` and the user config directory, both process-wide.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Holds [`super::EPOCH_GATE`] exclusively for as long as it lives, and marks this thread as the holder so the
/// real `lock_vault` these tests run (on this thread's own runtime) can bump the epoch without waiting for itself.
struct EpochGate(#[allow(dead_code)] std::sync::RwLockWriteGuard<'static, ()>);

impl EpochGate {
    fn hold() -> Self {
        let guard = super::EPOCH_GATE.write().unwrap_or_else(|e| e.into_inner());
        super::HOLDS_EPOCH_GATE.with(|held| held.set(true));
        Self(guard)
    }
}

impl Drop for EpochGate {
    fn drop(&mut self) {
        super::HOLDS_EPOCH_GATE.with(|held| held.set(false));
    }
}

/// The test gate itself: while a test holds the epoch gate, another thread's `bump_vault_epoch()` waits, so the
/// epoch a popover test reads cannot move under it; when the gate is released the bump goes through.
#[test]
fn a_test_that_holds_the_epoch_gate_is_not_moved_by_a_bump_on_another_thread() {
    use std::sync::atomic::Ordering;
    let gate = EpochGate::hold();
    let before = super::VAULT_EPOCH.load(Ordering::SeqCst);
    let bumper = std::thread::spawn(super::bump_vault_epoch);
    std::thread::sleep(std::time::Duration::from_millis(150));
    assert_eq!(
        super::VAULT_EPOCH.load(Ordering::SeqCst),
        before,
        "the epoch did not move while the gate was held"
    );
    assert!(
        !bumper.is_finished(),
        "the bump on the other thread is waiting for the gate"
    );
    // The holder's own bump (the real `lock_vault` these tests run) does not wait for itself.
    super::bump_vault_epoch();
    assert_eq!(super::VAULT_EPOCH.load(Ordering::SeqCst), before + 1);
    drop(gate);
    bumper.join().unwrap();
    // (At least: once the gate is released, other tests' waiting bumps go through as well.)
    assert!(
        super::VAULT_EPOCH.load(Ordering::SeqCst) >= before + 2,
        "released: the waiting bump went through"
    );
}

/// Point the command at `api` and at an empty config directory for the duration of `f`.
fn with_isolated_env<T>(api_base: &str, f: impl FnOnce() -> T) -> T {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _epoch = EpochGate::hold();
    let config_home = tempfile::tempdir().unwrap();
    let saved: Vec<(&str, Option<std::ffi::OsString>)> = ["BB_API_BASE", "XDG_CONFIG_HOME", "HOME"]
        .iter()
        .map(|k| (*k, std::env::var_os(k)))
        .collect();
    // SAFETY: serialized by ENV_LOCK; every exit path below restores the saved values.
    unsafe {
        std::env::set_var("BB_API_BASE", api_base);
        std::env::set_var("XDG_CONFIG_HOME", config_home.path());
        std::env::set_var("HOME", config_home.path());
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    for (key, value) in saved {
        // SAFETY: same guard as above.
        match value {
            Some(v) => unsafe { std::env::set_var(key, v) },
            None => unsafe { std::env::remove_var(key) },
        }
    }
    result.unwrap_or_else(|e| std::panic::resume_unwind(e))
}

fn mock_app_with_account(signed_in: bool, unlocked: bool) -> tauri::App<tauri::test::MockRuntime> {
    let app = tauri::test::mock_app();
    app.manage(AppState::default());
    app.manage(popover_data::PopoverRuntime::default());
    let state = app.state::<AppState>();
    synthesize_single_account(&*state, account::AccountId("acct-test".into()));
    if unlocked {
        let acct = state.active_account().unwrap();
        *acct.session.lock().unwrap() = Some(Session {
            token: "tok-abc".into(),
            master_key: [7u8; 32],
            email: Some("sam@example.eu".into()),
        });
    }
    if signed_in {
        *state.auth_present.lock().unwrap() = true;
    }
    app
}

fn run_command(app: &tauri::App<tauri::test::MockRuntime>) -> Result<serde_json::Value, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime
        .block_on(popover_snapshot(
            app.state::<AppState>(),
            app.state::<popover_data::PopoverRuntime>(),
            None,
        ))
        .map(|snapshot| serde_json::to_value(snapshot).unwrap())
}

const USAGE: &str = "{\"used_bytes\":84300000000,\"quota_bytes\":200000000000}";

/// Spec 2026-10-06: the snapshot reads the reconciler's published view from its own app's
/// `AppState` and never asks the OS, so a test installs the view it wants on the mock app.
/// Each app has its own, so nothing leaks between tests (the old probe was a process-wide
/// static that needed scoped guards).
fn set_finder_view(
    app: &tauri::App<tauri::test::MockRuntime>,
    setup: crate::surfaces::phase::FinderSetup,
    reason: Option<crate::surfaces::phase::FinderFailureReason>,
) {
    let view = finder_setup::driver::FinderSetupView {
        setup,
        reason,
        ..finder_setup::driver::FinderSetupView::initial(finder_setup::launch_location::LaunchLocation::Applications)
    };
    let _ = app
        .state::<AppState>()
        .finder_setup
        .set(finder_setup::driver::FinderSetupHandle::fixed(view));
}

#[test]
fn the_real_command_reads_the_session_the_engine_status_and_the_storage_api() {
    let api = LoopbackApi::start("200 OK", USAGE);
    with_isolated_env(&api.base, || {
        // The Finder leg is the reconciler's view, installed on the mock app, so it is
        // deterministic on every machine; the assertions still exercise the real phase
        // assembly.
        let app = mock_app_with_account(true, true);
        set_finder_view(&app, crate::surfaces::phase::FinderSetup::Ready, None);

        let first = run_command(&app).expect("the command runs");
        assert_eq!(first["phase"], "synced");
        assert_eq!(first["account"]["email"], "sam@example.eu");
        assert_eq!(first["account"]["vault_unlocked"], true);
        assert_eq!(first["storage"]["used_bytes"], 84_300_000_000_i64);
        assert_eq!(first["storage"]["quota_bytes"], 200_000_000_000_i64);
        assert_eq!(first["storage"]["stale"], false);
        assert_eq!(
            first["finder"]["setup"], "ready",
            "an installed Finder state reads as ready"
        );
        // The same shape the fixture validates.
        let shared: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/popover-snapshot.synced.json")).unwrap();
        for key in shared.as_object().unwrap().keys() {
            assert!(first.get(key).is_some(), "the command's snapshot lacks `{key}`");
        }
        assert_eq!(first.as_object().unwrap().len(), shared.as_object().unwrap().len());

        let requests = api.requests();
        assert_eq!(requests.len(), 1, "one call to the billing API");
        let raw = requests[0].to_ascii_lowercase();
        assert!(raw.starts_with("get /api/v1/billing/usage"), "{raw}");
        assert!(raw.contains("authorization: bearer tok-abc"), "{raw}");

        // Twenty more opens inside the TTL: the cache answers, the API is not called.
        for _ in 0..20 {
            assert_eq!(run_command(&app).unwrap()["storage"]["used_bytes"], 84_300_000_000_i64);
        }
        assert_eq!(api.requests().len(), 1, "21 snapshots, 1 fetch");

        // The engine says offline: the phase follows, still no extra call.
        let runtime = app.state::<popover_data::PopoverRuntime>();
        runtime.status.lock().unwrap().apply(&serde_json::json!({
            "state": "offline", "reason": { "code": "connect", "detail": null }, "last_tick_ok_at": 4242
        }));
        let offline = run_command(&app).unwrap();
        assert_eq!(offline["phase"], "offline");
        assert_eq!(
            offline["reason"],
            serde_json::json!({ "code": "connect", "detail": null })
        );
        assert_eq!(offline["engine"]["last_tick_ok_at"], 4242);

        // The user pauses: pause outranks offline.
        let state = app.state::<AppState>();
        state
            .active_account()
            .unwrap()
            .sync_paused
            .store(true, Ordering::Relaxed);
        assert_eq!(run_command(&app).unwrap()["phase"], "paused");

        // A session change (unlock, sign-in) refetches even inside the TTL.
        bump_vault_epoch();
        run_command(&app).unwrap();
        assert_eq!(api.requests().len(), 2, "a new session refetches the storage summary");
    });
}

#[test]
fn lock_then_unlock_refetches_the_storage_summary() {
    // The real `lock_vault` and the real unlock install (`install_unlocked_session`, the part of
    // `unlock_vault` after the Keychain read, which has no Linux backend), with the real
    // snapshot command between them. Nothing bumps the epoch by hand.
    let api = LoopbackApi::start("200 OK", USAGE);
    with_isolated_env(&api.base, || {
        let app = mock_app_with_account(true, true);
        // A reconciler stand-in that confirms the lock: on a Mac a lock it cannot confirm is not a
        // success (Task 9). Its view is the same one `set_finder_view` installs.
        let (handle, finder_events) =
            finder_setup::driver::FinderSetupHandle::for_test(finder_setup::driver::FinderSetupView {
                setup: crate::surfaces::phase::FinderSetup::Ready,
                ..finder_setup::driver::FinderSetupView::initial(
                    finder_setup::launch_location::LaunchLocation::Applications,
                )
            });
        let _ = app.state::<AppState>().finder_setup.set(handle);
        assert_eq!(run_command(&app).unwrap()["storage"]["used_bytes"], 84_300_000_000_i64);
        assert_eq!(run_command(&app).unwrap()["phase"], "synced");
        assert_eq!(api.requests().len(), 1, "two snapshots inside the TTL, 1 fetch");

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime
            .block_on(async {
                tokio::spawn(super::confirm_every_finder_event(finder_events));
                lock_vault(app.state::<AppState>()).await
            })
            .expect("the real lock runs");
        let locked = run_command(&app).unwrap();
        // (Not the phase: after a lock `auth_present` follows the real credential store, which
        // is empty on a Linux runner and the developer's own on a Mac.)
        assert_eq!(locked["account"]["vault_unlocked"], false);
        assert_eq!(
            locked["storage"],
            serde_json::Value::Null,
            "no storage figure while locked"
        );
        assert_eq!(api.requests().len(), 1, "a locked snapshot calls nothing");

        let acct = app.state::<AppState>().active_account().unwrap();
        install_unlocked_session(
            &SessionWrite::for_test(),
            &acct,
            Session {
                token: "tok-abc".into(),
                master_key: [7u8; 32],
                email: Some("sam@example.eu".into()),
            },
        )
        .unwrap();
        let unlocked = run_command(&app).unwrap();
        assert_eq!(unlocked["storage"]["used_bytes"], 84_300_000_000_i64);
        assert_eq!(
            api.requests().len(),
            2,
            "an unlock inside the 5 minute TTL refetches: the pre-lock figure is not served"
        );
    });
}

#[test]
fn the_real_command_never_calls_the_api_for_a_signed_out_or_locked_account() {
    let api = LoopbackApi::start("200 OK", USAGE);
    with_isolated_env(&api.base, || {
        let signed_out = mock_app_with_account(false, false);
        let s = run_command(&signed_out).unwrap();
        assert_eq!(s["phase"], "signed_out");
        assert_eq!(s["storage"], serde_json::Value::Null);

        let locked = mock_app_with_account(true, false);
        let l = run_command(&locked).unwrap();
        assert_eq!(l["phase"], "locked");
        assert_eq!(l["storage"], serde_json::Value::Null);
        assert_eq!(
            l["activity"],
            serde_json::json!([]),
            "no file names while the vault is locked"
        );

        assert_eq!(api.requests().len(), 0, "0 API calls without an unlocked session");
    });
}

#[test]
fn the_real_command_survives_a_storage_api_that_answers_500() {
    let api = LoopbackApi::start("500 Internal Server Error", "{}");
    with_isolated_env(&api.base, || {
        let app = mock_app_with_account(true, true);
        set_finder_view(&app, crate::surfaces::phase::FinderSetup::Ready, None);
        let snapshot = run_command(&app).expect("a failing usage call must not fail the whole snapshot");
        assert_eq!(snapshot["phase"], "synced");
        assert_eq!(snapshot["storage"], serde_json::Value::Null, "no number is invented");
        assert_eq!(api.requests().len(), 1);
    });
}

#[test]
fn without_a_reconciler_macos_reports_missing_and_other_platforms_ready() {
    let api = LoopbackApi::start("200 OK", USAGE);
    with_isolated_env(&api.base, || {
        let app = mock_app_with_account(true, true);
        let snapshot = run_command(&app).expect("the command runs");
        let expected = if cfg!(target_os = "macos") { "missing" } else { "ready" };
        assert_eq!(snapshot["finder"]["setup"], expected);
    });
}

/// A FAILED Finder view reaches the snapshot, with its typed reason, through the real
/// command: the phase mapping is asserted against the REAL command, not only
/// `popover_data::assemble` unit tests. macOS-only by construction: off macOS the snapshot
/// forces `ready` whatever any view says.
#[cfg(target_os = "macos")]
#[test]
fn a_failed_view_reaches_the_snapshot_with_its_reason() {
    let api = LoopbackApi::start("200 OK", USAGE);
    with_isolated_env(&api.base, || {
        let app = mock_app_with_account(true, true);
        set_finder_view(
            &app,
            crate::surfaces::phase::FinderSetup::Failed,
            Some(crate::surfaces::phase::FinderFailureReason::FolderTaken),
        );
        let snapshot = run_command(&app).expect("the command runs");
        assert_eq!(snapshot["phase"], "finder_failed");
        assert_eq!(snapshot["finder"]["setup"], "failed");
        assert_eq!(snapshot["finder"]["reason"], "folder_taken");
        assert_eq!(snapshot["finder"]["reason_line"], "reason: folder_taken");
        assert_eq!(
            snapshot["storage"]["used_bytes"], 84_300_000_000_i64,
            "a Finder failure does not hide the storage figure"
        );
    });
}

#[test]
fn the_real_status_listener_keeps_old_consumers_on_the_old_state_and_feeds_the_popover_the_new_one() {
    use tauri::Emitter;
    let app = mock_app_with_account(true, true);
    attach_tray_status_listener(app.handle());
    let runtime = app.state::<popover_data::PopoverRuntime>();
    let state = app.state::<AppState>();
    let engine = |state: &AppState| state.active_account().unwrap().engine_state.lock().unwrap().clone();

    // What `runner::run_status_pulse` emits while a file moves.
    app.handle()
        .emit(
            "engine-status",
            serde_json::json!({
                "state": "syncing", "legacy_state": "idle", "sync_root": null, "error": null,
                "files_remaining": 14, "bytes_total": 100, "bytes_done": 40,
                "last_tick_ok_at": 1234, "reason": null
            }),
        )
        .unwrap();
    // `sync_status` (old UI) still reads `idle`; the popover's mirror reads `syncing`.
    assert_eq!(engine(&state), "idle");
    {
        let view = runtime.status.lock().unwrap();
        assert_eq!(view.state, "syncing");
        assert_eq!(view.files_remaining, Some(14));
        assert_eq!(view.bytes_done, Some(40));
        assert_eq!(view.last_tick_ok_at, Some(1234));
    }

    // A link that is down, after a tick that returned Ok: old consumers still say idle.
    app.handle()
        .emit(
            "engine-status",
            serde_json::json!({
                "state": "offline", "legacy_state": "idle", "sync_root": null, "error": null,
                "files_remaining": null, "bytes_total": null, "bytes_done": null,
                "last_tick_ok_at": 1234, "reason": { "code": "connect", "detail": null }
            }),
        )
        .unwrap();
    assert_eq!(engine(&state), "idle");
    assert_eq!(runtime.status.lock().unwrap().state, "offline");
    assert_eq!(engine_status::sync_status_engine(&engine(&state)), "running");

    // The pause toggle sends a bare `{state}`: it is its own legacy value.
    app.handle()
        .emit("engine-status", serde_json::json!({ "state": "paused" }))
        .unwrap();
    assert_eq!(engine(&state), "paused");
    assert_eq!(runtime.status.lock().unwrap().state, "paused");
    assert_eq!(
        runtime.status.lock().unwrap().last_tick_ok_at,
        Some(1234),
        "a pause keeps the last check"
    );
}

/// The same command and the same engine HTTP client, against a REAL local API. Ignored by
/// default (needs a running server and a session token); the evidence run is in
/// `/home/user/evidence/1683-s2/real-api/`.
///
/// ```text
/// BB_REAL_API_BASE=http://127.0.0.1:3001 BB_REAL_API_TOKEN=<session token> \
///   cargo test --lib real_api -- --ignored --nocapture
/// ```
#[test]
#[ignore = "needs a running local API: BB_REAL_API_BASE and BB_REAL_API_TOKEN"]
fn real_api_the_command_and_the_engine_client_against_a_real_server() {
    let base = std::env::var("BB_REAL_API_BASE").expect("BB_REAL_API_BASE");
    let token = std::env::var("BB_REAL_API_TOKEN").expect("BB_REAL_API_TOKEN");
    with_isolated_env(&base, || {
        let app = mock_app_with_account(true, true);
        // Swap in the real session token.
        let state = app.state::<AppState>();
        state
            .active_account()
            .unwrap()
            .session
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .token = token.clone();
        let snapshot = run_command(&app).expect("the command runs against the real API");
        println!(
            "real API snapshot: {}",
            serde_json::to_string_pretty(&snapshot).unwrap()
        );
        assert_eq!(snapshot["phase"], "synced");
        let used = snapshot["storage"]["used_bytes"]
            .as_i64()
            .expect("used_bytes from the real API");
        let quota = snapshot["storage"]["quota_bytes"]
            .as_i64()
            .expect("quota_bytes from the real API");
        assert!(used >= 0, "used {used}");
        assert!(quota >= 0, "quota {quota}");
        assert_eq!(snapshot["storage"]["stale"], false);

        // A wrong token: the server answers 401, which is not a storage summary.
        let bad = mock_app_with_account(true, true);
        let bad_state = bad.state::<AppState>();
        // Bound first so the assignment below carries no string literal for the
        // repo's secret scanner to flag (a fixture, not a credential; value unchanged).
        let unrecognised = "not-a-real-token";
        bad_state
            .active_account()
            .unwrap()
            .session
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .token = unrecognised.into();
        let refused = run_command(&bad).expect("a 401 must not fail the snapshot");
        assert_eq!(refused["storage"], serde_json::Value::Null);

        // The engine's own HTTP client: a real `GET /sync/ops` is an answer, so the link
        // monitor records a check and no failure; a 401 is also an answer, not a link failure.
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let good = api_client::ApiClient::new(base.clone(), token.clone(), [0u8; 32]);
        let monitor = good.link();
        let ops = runtime.block_on(good.sync_ops(0));
        println!(
            "real GET /sync/ops: {:?}",
            ops.as_ref().map(|o| o.ops.len()).map_err(|e| e.to_string())
        );
        let seen = monitor.snapshot();
        assert!(seen.last_ok_at.is_some(), "a real answer records a check: {ops:?}");
        assert_eq!(seen.failure, None);
        let unauthorised = api_client::ApiClient::new(base.clone(), "not-a-real-token".into(), [0u8; 32]);
        let monitor = unauthorised.link();
        let err = runtime
            .block_on(unauthorised.sync_ops(0))
            .expect_err("a wrong token is refused");
        println!("real GET /sync/ops with a wrong token: {err}");
        assert!(err.to_string().contains("401"), "{err}");
        assert_eq!(monitor.snapshot().failure, None, "a 401 is the server answering");
    });
}

/// Task 1919: the two region COMMANDS the Settings windows invoke (`account_region`, `account_set_region`),
/// run through their real handlers against a REAL local API. Ignored by default.
///
/// ```text
/// BB_REAL_API_BASE=http://127.0.0.1:39191 BB_REAL_API_TOKEN=<session token> \
///   cargo test --lib real_api_region_commands -- --ignored --nocapture
/// ```
#[test]
#[ignore = "needs a running local API with de/fi pools: BB_REAL_API_BASE and BB_REAL_API_TOKEN"]
fn real_api_region_commands_list_choose_and_refuse() {
    let base = std::env::var("BB_REAL_API_BASE").expect("BB_REAL_API_BASE");
    let token = std::env::var("BB_REAL_API_TOKEN").expect("BB_REAL_API_TOKEN");
    assert!(
        base.starts_with("http://127.0.0.1:") || base.starts_with("http://localhost:"),
        "this test only ever talks to a local API, got {base}"
    );
    with_isolated_env(&base, || {
        let app = mock_app_with_account(true, true);
        app.state::<AppState>()
            .active_account()
            .unwrap()
            .session
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .token = token.clone();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let listed = runtime
            .block_on(account_region(app.state::<AppState>()))
            .expect("account_region against the real API");
        let ipc = serde_json::to_value(&listed).unwrap();
        println!("account_region IPC: {}", serde_json::to_string(&ipc).unwrap());
        let ids: Vec<&str> = listed.choices.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["europe", "fi", "de", "nl"]);
        assert!(!ipc.to_string().contains("provider"));
        assert!(!ipc.to_string().contains("city"), "no city reaches the IPC");

        let saved = runtime
            .block_on(account_set_region(
                app.state::<AppState>(),
                Some("fi".into()),
                Some(false),
            ))
            .expect("account_set_region fi");
        assert_eq!(saved.preferred_region.as_deref(), Some("fi"));
        let read_back = runtime.block_on(account_region(app.state::<AppState>())).unwrap();
        assert_eq!(read_back.preferred_region.as_deref(), Some("fi"));

        let refused = runtime
            .block_on(account_set_region(
                app.state::<AppState>(),
                Some("nl".into()),
                Some(false),
            ))
            .expect_err("nl is coming soon");
        println!("account_set_region nl -> {refused}");
        assert!(refused.contains("region_unavailable"), "{refused}");
        let still = runtime.block_on(account_region(app.state::<AppState>())).unwrap();
        assert_eq!(
            still.preferred_region.as_deref(),
            Some("fi"),
            "a refusal changes nothing"
        );

        let cleared = runtime
            .block_on(account_set_region(
                app.state::<AppState>(),
                Some("europe".into()),
                Some(false),
            ))
            .expect("account_set_region europe");
        assert_eq!(cleared.preferred_region.as_deref(), Some("europe"));
    });
}

/// The engine's real tick against a real server that then goes away. `sync_tick` swallows a
/// failed `GET /sync/ops` and returns `Ok`, so only the link monitor can say "offline".
/// Ignored by default: run it with the API up, and stop the API while the second phase waits.
///
/// ```text
/// BB_REAL_API_BASE=http://127.0.0.1:3901 BB_REAL_API_TOKEN=<token> \
///   cargo test --lib real_api_a_tick -- --ignored --nocapture   # then stop the API
/// ```
#[test]
#[ignore = "needs a running local API that the operator stops mid-test"]
fn real_api_a_tick_is_synced_then_offline_when_the_server_goes_away() {
    use engine_status::{Down, Outcome, tick_outcome};
    let base = std::env::var("BB_REAL_API_BASE").expect("BB_REAL_API_BASE");
    let token = std::env::var("BB_REAL_API_TOKEN").expect("BB_REAL_API_TOKEN");
    let dir = tempfile::tempdir().unwrap();
    let db = Arc::new(state_db::StateDb::open(dir.path().join("state.db")).unwrap());
    let api = Arc::new(api_client::ApiClient::new(base.clone(), token, [0u8; 32]));
    let link = api.link();
    let bridge = engine_bridge::EngineBridge::new(db, api);
    // A multi-thread runtime, like the engine's: it keeps polling the pooled connection
    // between ticks, so it notices the server closing it (a runtime that is only ever
    // driven inside `block_on` would write the next request to a dead socket and get a
    // reset instead of a refused connection).
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let now = || now_unix_seconds();

    // Phase 1: the server is up. The first tick bootstraps from `GET /sync/snapshot`.
    let started = now();
    let first = runtime.block_on(engine_bridge::sync_tick(&bridge, dir.path()));
    println!(
        "tick 1 (server up): {:?}",
        first.as_ref().map(|c| c.len()).map_err(|e| e.to_string())
    );
    let outcome = tick_outcome(first.as_ref().map(|_| ()).map_err(|e| e), &link.snapshot(), started);
    println!("tick 1 outcome: {outcome:?}; link: {:?}", link.snapshot());
    assert_eq!(outcome, Outcome::Synced);
    assert!(link.snapshot().last_ok_at.is_some());

    // Phase 2: wait until the server has really gone (connection refused).
    let addr = base.trim_start_matches("http://").to_string();
    let gone_by = std::time::Instant::now() + std::time::Duration::from_secs(60);
    println!("WAITING FOR THE API AT {addr} TO STOP");
    while std::net::TcpStream::connect(&addr).is_ok() {
        assert!(
            std::time::Instant::now() < gone_by,
            "the operator never stopped the API"
        );
        runtime.block_on(async { tokio::time::sleep(std::time::Duration::from_millis(200)).await });
    }
    // Let the runtime see the pooled connection close, as the engine's does between ticks.
    runtime.block_on(async { tokio::time::sleep(std::time::Duration::from_millis(500)).await });
    let started = now();
    let second = runtime.block_on(engine_bridge::sync_tick(&bridge, dir.path()));
    println!(
        "tick 2 (server gone): {:?}",
        second.as_ref().map(|c| c.len()).map_err(|e| e.to_string())
    );
    let outcome = tick_outcome(second.as_ref().map(|_| ()).map_err(|e| e), &link.snapshot(), started);
    println!("tick 2 outcome: {outcome:?}; link: {:?}", link.snapshot());
    assert!(
        matches!(
            outcome,
            Outcome::Down {
                down: Down::Offline(_),
                ..
            }
        ),
        "a tick that could not reach the server must be offline, got {outcome:?}"
    );
}

#[test]
fn the_real_command_reports_an_expired_session_before_anything_else() {
    let api = LoopbackApi::start("200 OK", USAGE);
    with_isolated_env(&api.base, || {
        let app = mock_app_with_account(true, true);
        let state = app.state::<AppState>();
        let acct = state.active_account().unwrap();
        for _ in 0..3 {
            acct.auth_health
                .note_result(Some(&anyhow::anyhow!("HTTP 401 Unauthorized: stale token")));
        }
        assert_eq!(run_command(&app).unwrap()["phase"], "session_ended");
    });
}
