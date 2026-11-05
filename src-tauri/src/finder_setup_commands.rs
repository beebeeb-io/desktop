//! `finder_setup_commands` items split out of `lib.rs`.

use super::*;

/// Spec §5.5 step 2 (macOS): make sure the engine runs before `addDomain`, because the extension talks
/// to the engine's socket. `Ok(true)`: this call started it, so the check must stop it if it fails;
/// `Ok(false)`: one was already running.
///
/// Only the wait for the engine slot is under a time limit. Once the engine is spawned nothing may
/// drop this future: the `started` answer would be lost and a failed check would never stop the
/// engine it started. The socket wait after it has its own 3 s bound.
#[cfg(target_os = "macos")]
pub(super) async fn start_check_engine(
    app: tauri::AppHandle,
    state: &AppState,
    acct: &account::AccountRuntime,
    root: PathBuf,
    slot_limit: std::time::Duration,
) -> Result<bool, finder_setup::error::FpError> {
    use finder_setup::error::{FpError, app_code};
    let (started, ipc_bind_error) = {
        // The slot first, then the keys under it (lead rulings T8-⚠lock 2 and 3): Lock and Sign-out clear
        // the session before they release the slot, so a check that waited for it finds no keys, and
        // nothing starts beside an engine whose stop was never confirmed. The keys are read, and the local
        // data bound to the account, inside `spawn_bound_engine`.
        let mut engine_slot = lock_engine_slot(acct, finder_setup::core::Op::StartEngine, slot_limit).await?;
        engine_start_refusal(acct).map_err(|error| FpError::app(app_code::ENGINE_START, error))?;
        // Rehydrate the persisted pause state before spawning. Best-effort: an unreadable config means not paused.
        rehydrate_sync_paused(&acct.sync_paused, false);
        if let Some(existing) = engine_slot.as_ref() {
            // Already running (a retry, or the restore started it): reuse its bind-status handle so a
            // REAL bind error is still seen instead of a silent timeout.
            (false, existing.ipc_bind_error_handle())
        } else {
            match spawn_bound_engine(app, state, acct, &mut engine_slot, root)
                .map_err(|error| FpError::app(app_code::ENGINE_START, error))?
            {
                EngineStart::Started => {}
                EngineStart::NoSession => {
                    return Err(FpError::app(app_code::ENGINE_START, ENGINE_START_NO_SESSION_FOR_FINDER));
                }
                EngineStart::FinderRemovalOwed => {
                    return Err(FpError::app(app_code::ENGINE_START, FINDER_REMOVAL_OWED_ERROR));
                }
            }
            let bind_error = engine_slot
                .as_ref()
                .map(|runner| runner.ipc_bind_error_handle())
                .ok_or_else(|| {
                    FpError::app(
                        app_code::ENGINE_START,
                        "the sync engine is not in its slot after it started",
                    )
                })?;
            (true, bind_error)
        }
    };

    if let Err(error) = wait_for_file_provider_ipc_ready(ipc_bind_error).await {
        if started && let Err(stop) = stop_check_engine(acct, slot_limit).await {
            tracing::error!(%stop, "finder setup: could not stop the engine whose socket never came up");
        }
        return Err(FpError::app(app_code::ENGINE_START, error));
    }
    Ok(started)
}

/// Spec §5.5 (macOS): stop the engine a failed check started. `Err` when the slot could not be taken in
/// time (the engine was not touched and is still tracked in its slot) or when `abort()` could not
/// confirm the engine task ended; the core never claims a stop that did not happen.
///
/// Only the wait for the engine slot is under a time limit. Once the engine is out of its slot this
/// future must run to the end: `abort()` gives up on its own after 5 s, but a future dropped inside it
/// detaches the engine task, which would keep running untracked, still holding the session token and
/// master key, while sign-out finds an empty slot and reports done.
#[cfg(target_os = "macos")]
pub(super) async fn stop_check_engine(
    acct: &account::AccountRuntime,
    slot_limit: std::time::Duration,
) -> Result<(), finder_setup::error::FpError> {
    use finder_setup::error::{FpError, app_code};
    let mut engine_slot = lock_engine_slot(acct, finder_setup::core::Op::StopEngine, slot_limit).await?;
    let Some(engine) = engine_slot.take() else {
        return Ok(());
    };
    if stop_engine_in_slot(acct, engine).await.task_confirmed() {
        return Ok(());
    }
    // The consumed handle leaves an empty slot, so the Bug-A2 flag is what keeps sign-out refused (and
    // says to restart) while the task may still be running; `stop_engine_in_slot` set it before the slot
    // is released.
    tracing::error!("finder setup: could not confirm the sync engine stopped; sign-out stays refused until a restart");
    Err(FpError::app(
        app_code::ENGINE_STOP_UNCONFIRMED,
        "the sync engine did not confirm it stopped",
    ))
}

pub(super) const FINDER_SETUP_MACOS_ONLY: &str = "Finder setup is only available on macOS.";

pub(super) fn finder_setup_state_impl(state: &AppState) -> Result<finder_setup::driver::FinderSetupView, String> {
    state
        .finder_setup
        .get()
        .map(|handle| handle.view())
        .ok_or_else(|| FINDER_SETUP_MACOS_ONLY.to_string())
}

/// What "Try again" says while the reconciler is held (after a sign-out or a Lock, until keys arrive): no check can add
/// Beebeeb to Finder before then. The quiet `Missing` row's own sentence (spec §5.2).
pub(super) const FINDER_SETUP_HELD: &str = "Beebeeb adds itself to Finder when you’re signed in and the vault is unlocked.";

/// "Try again" (§5.3 (4)). `Err` when there is no reconciler or it has stopped, which the frontend
/// shows as its existing "couldn't retry" toast (lead ruling 3), and while the reconciler is held, when the retry
/// could do nothing (area A M4): the sentence says what will add Beebeeb instead of a silent no-op.
pub(super) fn finder_setup_retry_impl(state: &AppState) -> Result<(), String> {
    let handle = state
        .finder_setup
        .get()
        .ok_or_else(|| FINDER_SETUP_MACOS_ONLY.to_string())?;
    if handle.is_held() {
        return Err(FINDER_SETUP_HELD.to_string());
    }
    handle.trigger(finder_setup::core::Trigger::TryAgain)
}

/// "Try again", against the server at `base_url` (lead ruling 5, Task 12): a session that has no email (so its engine
/// start cannot tell whose it is) asks the server first, so the check this asks for can start the engine the session
/// was refused. Then the reconciler's retry, whatever the answer was. A session with an email is already enough for the
/// engine start (`Identity::is_known`), so its retry never waits on the network (area A M8): that is exactly when the
/// network is slow or absent. An explicit action: not debounced, but never a second request while one is in flight
/// (fix round 1, item 4).
#[cfg(not(target_os = "windows"))]
pub(super) async fn finder_setup_try_again(state: &AppState, base_url: &str) -> Result<(), String> {
    if session_in_memory_has_no_email(state) {
        identify_unidentified_session(state, base_url, std::time::Instant::now(), IdentifyPace::Now).await;
    }
    finder_setup_retry_impl(state)
}

/// A session is in memory and has no email (its sign-in could not fetch the account record). No session, or a lock that
/// cannot be read: `false` (there is nothing to name).
#[cfg(not(target_os = "windows"))]
pub(super) fn session_in_memory_has_no_email(state: &AppState) -> bool {
    let Ok(acct) = state.active_account() else {
        return false;
    };
    acct.session
        .lock()
        .map(|guard| {
            guard
                .as_ref()
                .is_some_and(|session| session.email.as_deref().is_none_or(|email| email.trim().is_empty()))
        })
        .unwrap_or(false)
}

/// App activation, against the server at `base_url`, at `now` (lead ruling 5, Task 12; fix round 1, item 4): a session
/// that does not know its account's user id asks the server again, at most once a minute and never while a request is in
/// flight (every window focus lands here). `true` when it is named now, so the caller starts the engine it was refused.
#[cfg(not(target_os = "windows"))]
pub(super) async fn identify_on_app_activation(state: &AppState, base_url: &str, now: std::time::Instant) -> bool {
    identify_unidentified_session(state, base_url, now, IdentifyPace::Debounced).await
}

/// How often app activation may ask the server who an unidentified session is.
#[cfg(not(target_os = "windows"))]
pub(super) const IDENTIFY_ON_ACTIVATION_EVERY: std::time::Duration = std::time::Duration::from_secs(60);

/// Whether an identify request waits for [`IDENTIFY_ON_ACTIVATION_EVERY`] since the last one.
#[cfg(not(target_os = "windows"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum IdentifyPace {
    /// App activation.
    Debounced,
    /// "Try again" and the startup's second request: asked for once, so not debounced.
    Now,
}

/// One identify request in flight for an account (fix round 1, item 4). While it lives no other starts; dropping it ends
/// the flight.
#[cfg(not(target_os = "windows"))]
pub(super) struct IdentifyTicket<'a> {
    pub(super) acct: &'a account::AccountRuntime,
}

#[cfg(not(target_os = "windows"))]
impl Drop for IdentifyTicket<'_> {
    fn drop(&mut self) {
        let mut flight = self
            .acct
            .identify_flight
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        flight.in_flight = false;
    }
}

/// A ticket for one identify request at `now`, or `None`: one is in flight already, or (`Debounced`) the last one
/// started less than [`IDENTIFY_ON_ACTIVATION_EVERY`] ago.
#[cfg(not(target_os = "windows"))]
pub(super) fn begin_identify(
    acct: &account::AccountRuntime,
    now: std::time::Instant,
    pace: IdentifyPace,
) -> Option<IdentifyTicket<'_>> {
    let mut flight = acct
        .identify_flight
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if flight.in_flight {
        return None;
    }
    if pace == IdentifyPace::Debounced
        && flight
            .last_started
            .is_some_and(|at| now.saturating_duration_since(at) < IDENTIFY_ON_ACTIVATION_EVERY)
    {
        return None;
    }
    flight.in_flight = true;
    flight.last_started = Some(now);
    Some(IdentifyTicket { acct })
}

/// The active account's session, if it is in memory and does not know its account's user id, asks the server who it is
/// (see [`establish_session_identity`]), through the account's single flight (`pace`, see [`begin_identify`]). `true`
/// when it is named now. A session that is identified asks nothing.
#[cfg(not(target_os = "windows"))]
pub(super) async fn identify_unidentified_session(
    state: &AppState,
    base_url: &str,
    now: std::time::Instant,
    pace: IdentifyPace,
) -> bool {
    let Ok(acct) = state.active_account() else {
        return false;
    };
    // Captured before the session is read, so the Keychain write below refuses if any transition (a sign-out, a
    // lock, another sign-in) happened after this read.
    let turn = acct.session_generation();
    let email = match acct.session.lock() {
        Ok(guard) => match guard.as_ref() {
            Some(session) => session.email.clone(),
            None => return false,
        },
        Err(_) => return false,
    };
    if session_knows_its_user_id(&acct, email.as_deref()) {
        // Task 12 fix round 2, item 3: an email the naming could not write to the Keychain is written now, in a checked
        // turn (no request is needed: the session is named; the email is the one read from the session in memory
        // after `turn` was captured, so a checked turn means no transition happened since).
        if acct.keychain_email_pending.load(Ordering::SeqCst)
            && let Some(email) = email.as_deref()
            && let Ok(write) = check_session_turn(&acct, &turn)
        {
            write_session_email_to_keychain(&write, &acct, email);
        }
        return false;
    }
    let Some(_ticket) = begin_identify(&acct, now, pace) else {
        return false;
    };
    establish_session_identity(state, &acct, base_url, owner_record_dir().as_deref()).await
}

/// Name an unidentified session in the running app (`pace`), and when that worked, run the engine start it was refused
/// (the same start an unlock runs). Spawned, never awaited: app activation (debounced) and the startup restore's second
/// request (fix round 1, item 7), which never holds up the start.
#[cfg(not(target_os = "windows"))]
pub(super) async fn identify_then_start(app: tauri::AppHandle, pace: IdentifyPace) {
    let state = app.state::<AppState>();
    let named = match pace {
        IdentifyPace::Debounced => {
            identify_on_app_activation(&state, &runner::api_base_url(), std::time::Instant::now()).await
        }
        IdentifyPace::Now => {
            identify_unidentified_session(&state, &runner::api_base_url(), std::time::Instant::now(), pace).await
        }
    };
    if named {
        if let Err(error) = start_engine_unless_running(app.clone(), &state).await {
            tracing::warn!(%error, "the session was named after the fact, but the sync engine did not start");
        }
        // Area A M7: the reconciler may have failed this session's engine start before it was named, and it would show
        // that failure until "Try again". Keys are here, so it checks again by itself (a Ready domain is only confirmed).
        notify_finder(&state, finder_setup::core::Trigger::KeysArrived);
    }
}

/// What a naming in the background did with the engine (Task 12 fix round 2, item 2).
#[cfg(not(target_os = "windows"))]
#[derive(Debug, PartialEq, Eq)]
pub(super) enum AfterNaming {
    /// An engine was running: the naming only confirmed the account it runs for, so it keeps running.
    KeptRunning,
    /// The slot was empty (the start before was refused, for a session that did not know who it was): this start.
    Started(EngineStart),
    /// An engine was running, but the owner record of the local data and the named session both know a user id and
    /// the ids differ (fix round 3, M1): it was stopped through the one stop helper, and this start went through the
    /// binding.
    Restarted(EngineStart),
}

/// A naming in the background never restarts a running engine of the same account: it named only the session it fetched
/// for (the token is checked), so it confirmed the account that engine runs for. Only an empty slot gets a start, through
/// `start`. The one exception, an owner record that names another account by user id, is checked first by
/// [`keep_or_rebind_after_naming`].
#[cfg(not(target_os = "windows"))]
pub(super) fn keep_or_start_after_naming<'g>(
    engine_slot: &mut tokio::sync::MutexGuard<'g, Option<EngineRunner>>,
    start: impl FnOnce(&mut tokio::sync::MutexGuard<'g, Option<EngineRunner>>) -> Result<EngineStart, String>,
) -> Result<AfterNaming, String> {
    if engine_slot.is_some() {
        return Ok(AfterNaming::KeptRunning);
    }
    start(engine_slot).map(AfterNaming::Started)
}

/// Fix round 3, M1: the owner record of the local data and the named session both know a user id, and the ids differ.
#[cfg(not(target_os = "windows"))]
pub(super) fn owner_is_another_account_by_id(
    owner: Option<&account_binding::Identity>,
    named: &account_binding::Identity,
) -> bool {
    matches!(
        (owner.and_then(|owner| owner.user_id.as_deref()), named.user_id.as_deref()),
        (Some(owner), Some(named)) if owner != named
    )
}

/// The engine after a naming in the background (fix round 3, M1). A running engine is kept
/// ([`keep_or_start_after_naming`]) unless the owner record of the local data and the named session both know a user
/// id and the ids differ: then that engine runs over another account's data, so it is stopped through the one stop
/// helper ([`stop_engine_in_slot`]; an unconfirmed stop makes `start` refuse) and `start` goes through the binding,
/// which resets the data or refuses.
#[cfg(not(target_os = "windows"))]
pub(super) async fn keep_or_rebind_after_naming<'g>(
    acct: &account::AccountRuntime,
    engine_slot: &mut tokio::sync::MutexGuard<'g, Option<EngineRunner>>,
    owner: Option<&account_binding::Identity>,
    named: &account_binding::Identity,
    start: impl FnOnce(&mut tokio::sync::MutexGuard<'g, Option<EngineRunner>>) -> Result<EngineStart, String>,
) -> Result<AfterNaming, String> {
    if owner_is_another_account_by_id(owner, named)
        && let Some(running) = engine_slot.take()
    {
        stop_engine_in_slot(acct, running).await;
        return start(engine_slot).map(AfterNaming::Restarted);
    }
    keep_or_start_after_naming(engine_slot, start)
}

/// Who the session in memory is, from its email and the cached profile only (never its keys): what the naming named.
/// Read by the start after a naming (M1); no start site reads the session itself.
#[cfg(not(target_os = "windows"))]
pub(super) fn named_session_identity(acct: &account::AccountRuntime) -> account_binding::Identity {
    let email = acct
        .session
        .lock()
        .ok()
        .and_then(|guard| guard.as_ref().and_then(|session| session.email.clone()));
    let profile = acct.cached_profile.lock().ok().and_then(|guard| guard.clone());
    identity_of_session(email.as_deref(), profile.as_ref())
}

/// The engine start after a naming in the background (see [`keep_or_rebind_after_naming`]): like
/// `start_engine_if_possible`, but an engine of the same account that runs is kept, never stopped and respawned.
#[cfg(not(target_os = "windows"))]
pub(super) async fn start_engine_unless_running(app: tauri::AppHandle, state: &AppState) -> Result<(), String> {
    let Ok(cfg) = DesktopConfig::load() else {
        return Ok(());
    };
    let Some(root) = cfg.sync_root else {
        return Ok(());
    };
    let Ok(acct) = state.active_account() else {
        return Ok(());
    };
    let mut engine_slot = acct.engine.lock().await;
    // M1: the owner record and the named session, read under the slot. An owner that cannot be read compares with
    // nothing, so a running engine is kept, as before.
    let owner = owner_record_dir()
        .and_then(|dir| state_db_from_state_dir(&dir).ok().flatten())
        .and_then(|db| db.owner().ok().flatten())
        .filter(account_binding::Identity::is_known);
    let named = named_session_identity(&acct);
    let after = keep_or_rebind_after_naming(&acct, &mut engine_slot, owner.as_ref(), &named, |slot| {
        engine_start_refusal(&acct)?;
        rehydrate_sync_paused(&acct.sync_paused, cfg.pause_sync);
        spawn_bound_engine(app, state, &acct, slot, root)
    })
    .await?;
    tracing::info!(?after, "after the naming in the background");
    Ok(())
}

/// App activation in the running app ([`identify_then_start`], debounced). Spawned from the window-focus handler.
#[cfg(not(target_os = "windows"))]
pub(super) async fn on_app_activated(app: tauri::AppHandle) {
    identify_then_start(app, IdentifyPace::Debounced).await;
}

/// "Copy details" (§6.2): the one public `copy_details`, which reads the last 200 lifecycle-log
/// lines itself (lead ruling T5-b).
pub(super) fn finder_setup_copy_details_impl(state: &AppState) -> Result<String, String> {
    let view = finder_setup_state_impl(state)?;
    Ok(finder_setup::driver::copy_details(
        &view,
        real_app_version(),
        &macos_version_string(),
    ))
}

/// Spec §9: the reconciler's published state (replaces `finder_location_state` on macOS).
#[tauri::command]
pub(super) fn finder_setup_state(state: State<'_, AppState>) -> Result<finder_setup::driver::FinderSetupView, String> {
    finder_setup_state_impl(&state)
}

/// "Try again" (§5.3 (4)): a fresh retry budget. On macOS and Linux an unidentified session asks the server first
/// ([`finder_setup_try_again`]).
#[tauri::command]
pub(super) async fn finder_setup_retry(state: State<'_, AppState>) -> Result<(), String> {
    #[cfg(not(target_os = "windows"))]
    {
        finder_setup_try_again(&state, &runner::api_base_url()).await
    }
    #[cfg(target_os = "windows")]
    {
        finder_setup_retry_impl(&state)
    }
}

/// "Copy details" (§6.2): the text the frontend puts on the pasteboard.
#[tauri::command]
pub(super) fn finder_setup_copy_details(state: State<'_, AppState>) -> Result<String, String> {
    finder_setup_copy_details_impl(&state)
}

/// "Show in Finder" for `not_in_applications` (plan "Spec issues" 9): reveal the running app so
/// the person can drag it to Applications.
#[tauri::command]
pub(super) fn finder_setup_show_app(app: tauri::AppHandle) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let bundle = finder_setup::launch_location::current_bundle_path()
            .ok_or_else(|| "This copy of Beebeeb is not an app bundle.".to_string())?;
        // The opener's own text can name the bundle's path: it goes to the log, never to the caller.
        app.opener().reveal_item_in_dir(&bundle).map_err(|error| {
            tracing::warn!(%error, "finder setup: could not reveal the app bundle");
            "Could not show Beebeeb in Finder.".to_string()
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        Err("Only available on macOS.".to_string())
    }
}

/// `async`, with the read on the blocking pool: it waits up to 3 s for the bridge gate, which a plain
/// `#[tauri::command]` fn would do on the main thread (see [`on_the_blocking_pool`]).
#[tauri::command]
pub(super) async fn finder_domain_user_enabled() -> Result<Option<bool>, String> {
    on_the_blocking_pool(
        "Could not check whether Beebeeb is turned on in System Settings.",
        file_provider_domain_user_enabled,
    )
    .await
}

/// x-apple.systempreferences URL for the "Login Items & Extensions" pane (System
/// Settings -> General -> Login Items & Extensions on macOS 13+/Ventura's System
/// Settings rewrite onward), which lists File Providers -- the exact screen Issue 4's
/// fix (task 1524) needs the user to open to re-enable the Beebeeb domain. No
/// dedicated "File Providers" sub-anchor is documented anywhere, so this opens the
/// pane itself; the user finds "File Providers" as one of its listed sections (the
/// same path Guus's own screenshot showed: System Settings -> General -> Login Items
/// & Extensions -> File Providers).
///
/// Source (cross-checked 2026-09-28, no official Apple reference exists for these
/// URL schemes): <https://gist.github.com/rmcdongit/f66ff91e0dad78d4d6346a75ded4b751>
/// and <https://github.com/jaywcjlove/SystemSettings-URLs-macOS>, both listing
/// `x-apple.systempreferences:com.apple.LoginItems-Settings.extension` as this pane's
/// stable bundle id since System Settings replaced System Preferences (macOS 13
/// Ventura); still current as of macOS 15 Sequoia per the same sources.
pub(super) const MACOS_LOGIN_ITEMS_SETTINGS_URL: &str = "x-apple.systempreferences:com.apple.LoginItems-Settings.extension";
/// Fallback used only if the deep link above fails to launch (e.g. a future macOS
/// renames or removes that pane id) -- opens System Settings at its default pane.
pub(super) const MACOS_SYSTEM_SETTINGS_FALLBACK_URL: &str = "x-apple.systempreferences:";

/// `async`, with the open on the blocking pool: it waits up to 5 s for LaunchServices' answer, which a plain
/// `#[tauri::command]` fn would do on the main thread (see [`on_the_blocking_pool`]).
#[tauri::command]
pub(super) async fn open_login_items_and_extensions_settings() -> Result<(), String> {
    on_the_blocking_pool(
        "Could not open System Settings.",
        open_login_items_and_extensions_settings_blocking,
    )
    .await
}

pub(super) fn open_login_items_and_extensions_settings_blocking() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        // Through NSWorkspace, in this process (task 1885), and the answer is LaunchServices': a refusal is the
        // command's error, not a discarded one. No bridge gate: this touches no File Provider state, and it must
        // work exactly when the File Provider is stuck.
        match macos_workspace::open_url(MACOS_LOGIN_ITEMS_SETTINGS_URL) {
            Ok(()) => Ok(()),
            Err(primary) => {
                tracing::warn!(%primary, "could not open the Login Items pane; opening System Settings itself");
                macos_workspace::open_url(MACOS_SYSTEM_SETTINGS_FALLBACK_URL).map_err(|fallback| {
                    tracing::warn!(%fallback, "could not open System Settings");
                    fallback.redacted()
                })
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("Only available on macOS.".to_string())
    }
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
pub(super) fn finder_location_state() -> Result<FinderInstallState, String> {
    let mut cfg = HeldConfig::load()?;
    match file_provider_installed() {
        Ok(FileProviderStatusOutcome::Installed) => {
            if cfg.finder_install_status.as_deref() != Some("installed") {
                persist_finder_install_result(&mut cfg, true, None)?;
            }
            Ok(finder_install_state_from_config(&cfg, true, None))
        }
        Ok(FileProviderStatusOutcome::NotInstalled) => Ok(finder_install_state_from_config(&cfg, false, None)),
        Ok(FileProviderStatusOutcome::UserDisabled) => {
            // Issue 4's status-path leg (PR #63 Codex review, Onboarding.tsx:488): detected
            // live here, WITHOUT waiting for stabilization (macos_file_provider::status
            // short-circuits via the same decide_install_step used by install()) -- so this
            // is a fresh runtime observation, not a stale persisted one. Pass it through as
            // `runtime_error` using the SAME crafted message `install_finder_location`
            // persists, so `finder_install_state_from_config`'s existing
            // "runtime_error always wins over cfg" precedence gives it priority over
            // whatever is currently persisted (e.g. a stale "timeout" category from before
            // this fix), and `classify_finder_install_error` maps it back to
            // "user_disabled" identically to a fresh install attempt. A GENUINE
            // stabilization timeout (the domain is enabled but never comes up) still falls
            // through to the `Err(error)` arm below unchanged, so it still shows timeout
            // copy, never this one.
            Ok(finder_install_state_from_config(
                &cfg,
                false,
                Some(FINDER_USER_DISABLED_MESSAGE.to_string()),
            ))
        }
        Err(error) => Ok(finder_install_state_from_config(&cfg, false, Some(error))),
    }
}

/// Persist the chosen sync root and start the daemon when possible, but do not
/// claim Finder integration succeeded while the `.appex` is absent.
///
/// Windows and Linux only. On macOS Beebeeb adds itself to Finder (spec 2026-10-06, ruling R5);
/// the macOS variant below only refuses.
#[cfg(not(target_os = "macos"))]
#[tauri::command]
pub(super) async fn install_finder_location(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    path: Option<String>,
) -> Result<FinderInstallState, String> {
    #[cfg(target_os = "windows")]
    let _transition = SESSION_TRANSITION.lock().await;
    #[cfg(target_os = "windows")]
    {
        windows_cf::ensure_reactivation_allowed()?;
        SESSION_COMMANDS.generation()?;
    }

    let root = path
        .map(PathBuf::from)
        .unwrap_or_else(config::default_sync_root_suggestion);
    if !root.is_absolute() {
        return Err(format!("Finder location must be absolute: {}", root.display()));
    }
    config::ensure_directory(&root)?;

    let mut cfg = HeldConfig::load()?;
    // Spec section 7: the previous attempt's saved failure is cleared when a new attempt starts.
    begin_finder_install_attempt(&mut cfg)?;
    let started_pending_engine = start_engine_for_pending_finder_install(
        app.clone(),
        &state,
        root.clone(),
        #[cfg(target_os = "windows")]
        &_transition,
    )
    .await?;

    match install_file_provider_domain_blocking().await {
        Err(failure) => {
            stop_pending_finder_install_engine(&state, started_pending_engine).await;
            // Review M1 (round 2): the install's own cleanup removed a domain this attempt
            // added; a folder it kept is saved for the row and shown, like the rollback's. Task 1889: the
            // record is written onto the file as it is now (`cfg` is a read copy from before the slow work),
            // so the save just below cannot put an older row back.
            if let Some(location) = failure.kept_folder.as_deref() {
                finder_removal::remember_kept_folder_held(&mut cfg, location);
                show_preserved_files_alert(&app, Some(location));
            }
            // D1 (task 1683 slice 5): saved OR returned as an error, not both.
            return finder_install_failed(&mut cfg, failure.message);
        }
        Ok(FileProviderInstallOutcome::UserDisabled) => {
            // Issue 4: do not wait, do not treat this as a "Continue without install"
            // situation -- persist a distinct, classifiable reason so the frontend can
            // show the real cause and offer the System Settings deep link instead of a
            // generic error or a silent fall-through to no Finder integration at all.
            stop_pending_finder_install_engine(&state, started_pending_engine).await;
            persist_finder_install_result(&mut cfg, false, Some(FINDER_USER_DISABLED_MESSAGE.to_string()))?;
            return Ok(finder_install_state_from_config(&cfg, false, None));
        }
        Ok(FileProviderInstallOutcome::Installed) => {}
    }
    if let Err(error) = persist_sync_root_and_start_engine(
        app.clone(),
        &state,
        &mut cfg,
        root.clone(),
        #[cfg(target_os = "windows")]
        &_transition,
    )
    .await
    {
        stop_pending_finder_install_engine(&state, started_pending_engine).await;
        // Task 1882: `addDomain` also succeeds on a domain that already existed, so this
        // rollback can remove one that holds un-synced files. They are kept; the alert names
        // the folder. A failed rollback stays silent, as before.
        // Review I2 / task 1889: the record is written onto the file as it is now and into `cfg`, so the save
        // `finder_install_failed` makes below cannot overwrite it with a stale copy.
        // Review M2: a folder kept with a failed removal is surfaced too.
        let removal = remove_file_provider_domain_blocking().await;
        if let Some(location) = removal.map_or_else(
            |failure| failure.kept_location("add-to-finder rollback"),
            |removal| removal.kept_location("add-to-finder rollback"),
        ) {
            finder_removal::remember_kept_folder_held(&mut cfg, &location);
            show_preserved_files_alert(&app, Some(&location));
        }
        // D1 (task 1683 slice 5): saved OR returned as an error, not both.
        return finder_install_failed(&mut cfg, error);
    }
    persist_finder_install_result(&mut cfg, true, None)?;
    Ok(FinderInstallState {
        installed: true,
        path: finder_state_path(&cfg, true),
        status: "installed".to_string(),
        last_error: None,
        last_attempt_at: cfg.finder_install_last_attempt_at,
        reason_category: None,
    })
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
pub(super) async fn continue_without_finder_location(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    path: Option<String>,
) -> Result<FinderInstallState, String> {
    #[cfg(target_os = "windows")]
    let _transition = SESSION_TRANSITION.lock().await;
    #[cfg(target_os = "windows")]
    {
        windows_cf::ensure_reactivation_allowed()?;
        SESSION_COMMANDS.generation()?;
    }

    let root = path
        .map(PathBuf::from)
        .unwrap_or_else(config::default_sync_root_suggestion);
    if !root.is_absolute() {
        return Err(format!("Finder location must be absolute: {}", root.display()));
    }
    config::ensure_directory(&root)?;

    let mut cfg = HeldConfig::load()?;
    persist_sync_root_and_start_engine(
        app,
        &state,
        &mut cfg,
        root.clone(),
        #[cfg(target_os = "windows")]
        &_transition,
    )
    .await?;
    Ok(finder_install_state_from_config(&cfg, false, None))
}

/// macOS: Beebeeb adds itself to Finder (spec 2026-10-06, ruling R5). Registered so the command
/// table is the same on every platform; nothing on macOS calls it
/// (tests/finderSetupSourceContract.test.ts).
#[cfg(target_os = "macos")]
#[tauri::command]
pub(super) async fn install_finder_location(path: Option<String>) -> Result<FinderInstallState, String> {
    let _ = path;
    Err("On macOS Beebeeb adds itself to Finder. Use finder_setup_retry.".to_string())
}

/// macOS: there is no "continue without Finder" (spec §4: Finder setup is automatic). Registered
/// so the command table is the same on every platform; nothing on macOS calls it, and it never
/// reads or writes the install-era state (lead ruling 1).
#[cfg(target_os = "macos")]
#[tauri::command]
pub(super) async fn continue_without_finder_location(path: Option<String>) -> Result<FinderInstallState, String> {
    let _ = path;
    Err("Finder setup is automatic on macOS".to_string())
}

/// macOS: the reconciler's published view is `finder_setup_state`. Registered so the command
/// table is the same on every platform; nothing on macOS calls it.
#[cfg(target_os = "macos")]
#[tauri::command]
pub(super) fn finder_location_state() -> Result<FinderInstallState, String> {
    Err("On macOS use finder_setup_state.".to_string())
}

// ── Windows shell integration (Cloud Files) ───────────────────────────────────
//
// Windows mirror of the macOS Finder commands above. The Cloud Files sync root
// plays the role the macOS File Provider domain plays: registering it makes the
// sync folder show up in Explorer with on-demand (cloud-only) placeholders. We
// reuse `FinderInstallState` as the return type so the frontend only needs a
// type alias (`FinderInstallState` → `ShellIntegrationState`); the persisted
// `finder_install_*` config fields double as the shell-integration status store
// on Windows, so existing state helpers apply unchanged.
//
// `installed` here means "the Cloud Files sync root is registered with Windows."
// Registration is idempotent at the OS layer (re-registering the same path is a
// no-op), so these commands are safe to call repeatedly. The live registration
// performed by the engine runner (`windows_cf::connect_root`) and the one done
// here converge on the same OS state.

/// Report whether the Beebeeb Cloud Files sync root is registered with Windows.
/// Parallels `finder_location_state`. Returns `installed: true` once the sync
/// root has been registered (status persisted as "installed" in config).
#[tauri::command]
pub(super) fn windows_shell_integration_state() -> Result<FinderInstallState, String> {
    #[cfg(target_os = "windows")]
    {
        let cfg = DesktopConfig::load()?;
        // Treat a registered sync root as "installed". We persisted the
        // outcome of the last registration attempt under the finder_install_*
        // fields, so the state is reconstructed exactly like the macOS path.
        let installed = cfg.finder_install_status.as_deref() == Some("installed");
        Ok(finder_install_state_from_config(&cfg, installed, None))
    }
    #[cfg(not(target_os = "windows"))]
    {
        Err("Windows shell integration is only available on Windows.".to_string())
    }
}

/// Register the chosen sync root as a Windows Cloud Files sync root and start
/// the daemon. Parallels `install_finder_location`. On success the folder
/// appears in Explorer as a Beebeeb cloud-synced location.
#[tauri::command]
pub(super) async fn install_windows_shell_integration(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    path: Option<String>,
) -> Result<FinderInstallState, String> {
    #[cfg(target_os = "windows")]
    let _transition = SESSION_TRANSITION.lock().await;
    #[cfg(target_os = "windows")]
    {
        windows_cf::ensure_reactivation_allowed()?;
        SESSION_COMMANDS.generation()?;
    }

    #[cfg(target_os = "windows")]
    {
        let root = path
            .map(PathBuf::from)
            .unwrap_or_else(config::default_sync_root_suggestion);
        if !root.is_absolute() {
            return Err(format!("Sync root must be absolute: {}", root.display()));
        }
        config::ensure_directory(&root)?;

        let mut cfg = HeldConfig::load()?;

        // Register the Cloud Files sync root with Windows. Idempotent at the OS
        // layer. The engine runner re-registers + connects callbacks on spawn;
        // doing it here gives immediate feedback in onboarding before the engine
        // ticks.
        if let Err(error) = windows_cf::register_sync_root(&root) {
            let error = error.to_string();
            persist_finder_install_result(&mut cfg, false, Some(error.clone()))?;
            return Err(error);
        }

        // Write the Explorer SHELL registration (Status column + cloud/check
        // overlays + "Beebeeb" sidebar entry + "Free up space" menu) right after
        // the Win32 platform registration. PURELY ADDITIVE — the Win32 path above
        // already drives placeholders + hydration; this adds only the shell
        // metadata that makes the folder look like OneDrive in Explorer. The
        // engine runner re-runs it on spawn (windows_cf::connect_root), so a
        // re-login reconverges. Best-effort: a shell-registration failure must
        // NOT fail onboarding (placeholders + hydration still work without it),
        // so we log and continue rather than returning an error here.
        if let Err(error) = windows_cf::register_shell_sync_root(&root) {
            tracing::warn!(error = %error, "Explorer shell sync-root registration failed during onboarding; overlays/column/sidebar may be absent (hydration unaffected)");
        }

        // Persist the sync root and (re)start the engine, which will connect the
        // Cloud Files fetch callbacks and seed placeholders.
        if let Err(error) = persist_sync_root_and_start_engine(
            app,
            &state,
            &mut cfg,
            root.clone(),
            #[cfg(target_os = "windows")]
            &_transition,
        )
        .await
        {
            persist_finder_install_result(&mut cfg, false, Some(error.clone()))?;
            return Err(error);
        }

        persist_finder_install_result(&mut cfg, true, None)?;
        Ok(FinderInstallState {
            installed: true,
            path: Some(root.to_string_lossy().into_owned()),
            status: "installed".to_string(),
            last_error: None,
            last_attempt_at: cfg.finder_install_last_attempt_at,
            reason_category: None,
        })
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (app, state, path);
        Err("Windows shell integration is only available on Windows.".to_string())
    }
}

#[tauri::command]
pub(super) async fn reset_macos_integration(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<MacosIntegrationResetResult, String> {
    #[cfg(target_os = "windows")]
    let _transition = SESSION_TRANSITION.lock().await;
    #[cfg(target_os = "windows")]
    {
        windows_cf::ensure_reactivation_allowed()?;
        SESSION_COMMANDS.generation()?;
    }

    let mut warnings = Vec::new();
    #[cfg_attr(target_os = "macos", allow(unused_mut))]
    let mut cfg = HeldConfig::load()?;
    let sync_root_preserved = cfg.sync_root.as_ref().map(|path| path.to_string_lossy().into_owned());

    let queue = match state_db_for_config(&cfg) {
        Ok(Some(db)) => {
            let now = now_unix_seconds();
            match db.queue_diagnostics(now) {
                Ok(queue) => Some((db, queue)),
                Err(error) => {
                    warnings.push(format!("Could not read sync queue diagnostics: {error}"));
                    None
                }
            }
        }
        Ok(None) => None,
        Err(error) => {
            warnings.push(error);
            None
        }
    };

    let pending_operations_preserved = queue.as_ref().map(|(_, queue)| queue.queued).unwrap_or(0);

    let mut removed_cache_files = 0usize;
    let mut skipped_cache_files = 0usize;
    if let Some((db, queue)) = &queue {
        if queue.queued > 0 {
            warnings.push(format!(
                "Preserved {count} queued operation(s); disposable cache cleanup was skipped.",
                count = queue.queued
            ));
        } else {
            let mut cleared_paths = Vec::new();
            match db.disposable_unpinned_cache_paths() {
                Ok(paths) => {
                    for path in paths {
                        let path_buf = PathBuf::from(&path);
                        if !is_disposable_cache_path(&path_buf) {
                            skipped_cache_files += 1;
                            continue;
                        }
                        match std::fs::remove_file(&path_buf) {
                            Ok(()) => {
                                removed_cache_files += 1;
                                cleared_paths.push(path);
                            }
                            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                                cleared_paths.push(path);
                            }
                            Err(error) => {
                                skipped_cache_files += 1;
                                warnings.push(format!("Could not remove cache file {}: {error}", path_buf.display()));
                            }
                        }
                    }
                }
                Err(error) => warnings.push(format!("Could not list disposable cache files: {error}")),
            }
            if !cleared_paths.is_empty()
                && let Err(error) = db.clear_cache_metadata_for_paths(&cleared_paths, now_unix_seconds())
            {
                warnings.push(format!("Could not update cache metadata after cleanup: {error}"));
            }
        }
    }

    // The engine + its status string are per-account (decision 0800); the
    // `cfg`/`sync_root` reads in this handler stay on `DesktopConfig` (Group D).
    let acct = state.active_account()?;
    let RepairEngineStop {
        removed_socket,
        engine_stop_unconfirmed,
    } = stop_engine_for_repair(&acct, &mut warnings, remove_stale_ipc_socket).await;

    // Windows: strip the Explorer SHELL registration (nav-pane entry, Status
    // column, overlays) as part of a full integration reset — the analogue of
    // removing the Finder File Provider domain on macOS below. The Win32 Cloud
    // Files registration + placeholders are intentionally left intact (the reset
    // preserves the sync root + queued operations); this removes only the shell
    // chrome, which a subsequent install / login re-creates via connect_root.
    // Best-effort: a failure (incl. "not registered") is collected as a warning,
    // never fatal.
    #[cfg(target_os = "windows")]
    {
        if let Some(root) = cfg.sync_root.clone() {
            if let Err(error) = windows_cf::unregister_shell_sync_root(&root) {
                warnings.push(format!("Could not remove Explorer shell sync-root entry: {error}"));
            }
        }
    }

    // Spec §5.3 (6): Repair removes through the reconciler, which then runs one fresh check by itself,
    // so a signed-in Mac ends with Beebeeb back in Finder. The engine was stopped above, before the
    // reconciler is told: the fresh check starts it again with the keys still in memory.
    #[cfg(target_os = "macos")]
    let removal = finder_remove_for(&state, finder_setup::core::Trigger::Repair).await;
    #[cfg(not(target_os = "macos"))]
    let removal = remove_file_provider_domain_blocking().await;
    // Task 1882: the removal keeps un-synced files; their folder rides the
    // result to the Sync tab (spec 2026-10-09 §5). A failed removal is one warning, as before.
    let (removed_file_provider_domain, preserved_location) = finder_removal::repair_removal(removal, &mut warnings);
    // Review I2: saved for the Settings › Sync row. On macOS this command saves no config (the reconciler owns
    // the Finder state), so the folder is saved on its own, under the config-write lock; Windows and Linux
    // write it onto the file as it is now (task 1889), and into the copy this command loaded, which the save below
    // goes through.
    if let Some(location) = preserved_location.as_deref() {
        #[cfg(target_os = "macos")]
        save_kept_folder_for_row(&app, location);
        #[cfg(not(target_os = "macos"))]
        finder_removal::remember_kept_folder_held(&mut cfg, location);
    }

    let disabled_autostart = match app.autolaunch().is_enabled() {
        Ok(true) => match app.autolaunch().disable() {
            Ok(()) => true,
            Err(error) => {
                warnings.push(format!("Could not disable Start at login: {error}"));
                false
            }
        },
        Ok(false) => false,
        Err(error) => {
            warnings.push(format!("Could not read Start at login state: {error}"));
            false
        }
    };

    // Windows and Linux only: on macOS the reconciler owns the Finder state and nothing writes the
    // install-era keys (spec 2026-10-06 §6.1). Re-review P1 (task 1882): the removal above cannot be
    // undone, so a failed save still shows the folder.
    #[cfg(not(target_os = "macos"))]
    finish_repair_after_removal(
        &mut cfg,
        preserved_location.as_deref(),
        |cfg| persist_finder_install_result(cfg, false, None),
        |location| surface_kept_folder(&app, location),
    )?;

    Ok(MacosIntegrationResetResult {
        removed_file_provider_domain,
        preserved_location,
        disabled_autostart,
        removed_socket,
        removed_cache_files,
        skipped_cache_files,
        pending_operations_preserved,
        sync_root_preserved,
        engine_stop_unconfirmed,
        warnings,
    })
}

#[cfg(not(target_os = "macos"))]
pub(super) fn require_open_folder_root(path: Option<String>) -> Result<PathBuf, String> {
    path.filter(|root| !root.trim().is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| "Not configured on this PC yet".to_string())
}
