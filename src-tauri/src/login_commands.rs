//! `login_commands` items split out of `lib.rs`.

use super::*;

/// Everything the startup restore does before its engine start, against the server at `base_url` (production passes
/// [`runner::api_base_url`]; a test passes a loopback stand-in): read the Keychain, offer the upgrade's adoption what
/// it read, ask the server whether the stored token still works, and put the session in memory. Returns whether a
/// session is in memory now, so the caller starts an engine for it.
pub(super) async fn restore_stored_session(state: &AppState, acct: &AccountRuntime, base_url: &str) -> bool {
    // This restore is one session transition (lead ruling 1, Task 12): its generation is captured before anything is
    // read, and every write below happens in its turn. A Lock, a Sign-out or a sign-in that completes while the probe
    // waits ends the turn, and the restore then writes nothing, installs nothing and asks for no engine.
    let mut turn = acct.session_generation();
    // Don't clobber a session already installed this run (e.g. a fresh login
    // that landed before this startup task ran). The upgrade's adoption window is over either way.
    if acct.session.lock().map(|g| g.is_some()).unwrap_or(false) {
        adopt_unbound_local_data_at_startup(None);
        return false;
    }

    let email = acct.auth_email.lock().ok().and_then(|guard| guard.clone());
    let loaded = load_session_from_keychain(acct.id.as_str(), email);
    // R10: local data from before the account binding is adopted, once, by the account whose vault key this
    // computer holds, before the probe and before anything can start. With no vault key nobody is adopted and
    // the window shuts all the same. In this restore's turn: what was read is still what is stored.
    {
        let Ok(_write) = claim_session_write(acct, &mut turn) else {
            adopt_unbound_local_data_at_startup(None);
            return false;
        };
        adopt_unbound_local_data_at_startup(startup_adoption_candidate(&loaded).as_ref());
    }
    let session = match loaded {
        // Both token + master key present → vault is unlocked; resume.
        Ok(Some(session)) => session,
        // No stored session TOKEN at all (never signed in / logged out).
        // Leave onboarding to prompt for sign-in. Not an error.
        Ok(None) => return false,
        Err(error) => {
            // The token IS present but `unlock()` failed. Two very different cases:
            //   1. Key genuinely ABSENT — `AuthStoreError::NotFound` (Display:
            //      "secret not found"). This is the legitimate "this PC has your
            //      session but not your vault key" state; onboarding's recovery-
            //      phrase step must appear. Expected → log at debug, not warn.
            //   2. A real credential-store / backend failure (locked keychain,
            //      DPAPI error, deserialisation). Worth a warning so the user can
            //      see why auto-unlock didn't happen — but must NOT block startup.
            if error.contains("secret not found") {
                tracing::debug!(%error, "no vault key to auto-unlock at startup; recovery-phrase unlock required");
            } else {
                tracing::warn!(%error, "credential-store error while attempting auto-unlock at startup");
            }
            return false;
        }
    };

    // Definitive-401 auto sign-out (the dead-session trap fix): the restore
    // used to reinstall a stored token with NO server validation, while every
    // login path refuses while a session is installed — a revoked/expired
    // token trapped the user. ONE bounded probe decides: HTTP 401 → the
    // stored session is definitively dead → clear the keychain trio and boot
    // signed-out. ANYTHING else (network error, timeout, 5xx, other status)
    // fails OPEN — keep the session and current behavior; an offline user
    // must never be logged out by startup. Still inside the spawned startup
    // task: setup() is never blocked by this.
    let probe = probe_startup_session(base_url, &session.token).await;
    // Lead ruling, F9 review I-2, on every OS since spec D11 (task 1888): a token the server confirmed resumes the
    // queue below.
    let token_confirmed = matches!(probe, StartupSessionCheck::Authorized { .. });
    let probed = match probe {
        StartupSessionCheck::Unauthorized => {
            #[cfg(not(target_os = "windows"))]
            discard_unusable_startup_session(state, acct, session.email.clone(), &mut turn);
            #[cfg(target_os = "windows")]
            discard_unusable_startup_session(state, acct, session.email.clone());
            return false;
        }
        StartupSessionCheck::Authorized { profile } => profile,
        StartupSessionCheck::Inconclusive(reason) => {
            tracing::info!(
                %reason,
                "startup session probe inconclusive; failing open and keeping the stored session"
            );
            None
        }
    };

    let token = zeroize::Zeroizing::new(session.token.clone());
    let email = session.email.clone();
    {
        // Everything the restore puts in memory, in its turn: the session, the flags that mirror it, and what the
        // probe learned of the account.
        let Ok(write) = claim_session_write(acct, &mut turn) else {
            tracing::info!("the session changed while the startup probe waited; nothing restored");
            return false;
        };
        match acct.session.lock() {
            Ok(mut guard) => *guard = Some(session),
            Err(_) => {
                tracing::warn!("session mutex poisoned during startup restore");
                return false;
            }
        }
        set_auth_present(state, true);
        // Mirror `apply_session` / the live-unlock path: keep `auth_email` in step
        // with the restored session so the Account page shows the signed-in email
        // after an auto-unlock, instead of going blank until the next login. Only
        // overwrite when the restored session actually carries an email — never
        // clobber an already-known address with `None`.
        if email.is_some() {
            set_auth_email(state, email);
        }
        // R10 (lead rulings 4 and 5): the account the server named while the session still works. The session is
        // named after it (its user id is known from now on, so the binding compares ids), and an owner record of this
        // account written before ids were known is completed. The profile is cached either way.
        if let Some(profile) = probed {
            name_session_after_profile(&write, state, acct, &token, &profile, owner_record_dir().as_deref());
            if let Ok(mut cached) = acct.cached_profile.lock() {
                *cached = Some(profile);
            }
        }
        // Lead ruling, F9 review I-2: the server confirmed the stored token and the Keychain unlocked the vault, so
        // sync resumes as after a sign-in (the relaunch "Quit and reopen Beebeeb" asks for). A Mac makes the edits
        // paused for `auth` due before the engine's first tick; Windows and Linux after the engine's reconcile pass and
        // key check (spec D11, task 1888). Those held as `key_replaced` stay held.
        if token_confirmed {
            ask_to_resume_auth_paused(&write, acct);
        }
    }
    tracing::info!("vault auto-unlocked from credential store on startup");
    // A restored session the probe could not name (offline, or an unreadable answer) asks the server once more, but not
    // here (fix round 1, item 7): the caller does that without holding up the engine start.
    true
}

// UI cache epoch, independent of the credential-bearing command lease epoch.
pub(super) static UI_SESSION_REVISION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Bumped at every place the in-memory session is installed by a sign-in or recovery-phrase
/// unlock, and at every place it is cleared (lock, sign-out). The password unlock from the
/// Keychain (`install_unlocked_session`) does NOT bump: it is correct only because the lock that
/// came before it did. A new path that clears the session (an auth-expiry auto-lock, say) must
/// call `bump_vault_epoch()` itself, or the popover serves the pre-clear storage figure for up to
/// five minutes after the next unlock. `lock_then_unlock_refetches_the_storage_summary` runs the
/// real lock and the real unlock install and fails if the lock stops bumping.
///
/// Keys the popover's cached storage summary, so an unlock refetches it even when
/// `UI_SESSION_REVISION` does not move (a lock keeps `auth_present` true, and the email is the
/// same on the way back). Kept separate from `UI_SESSION_REVISION` on purpose: bumping that one
/// makes the existing UI refetch, and slice 2 changes nothing a user sees.
pub(super) static VAULT_EPOCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Unit tests only: a test that reads the epoch (the popover's storage cache is keyed by it) holds this gate
/// exclusively while it runs, and every other test's `bump_vault_epoch()` waits for it. The epoch is one static
/// for the whole test process, and any Lock, Sign-out or sign-in test that ran in parallel moved it under the
/// popover tests' feet (a fetch count that was 1 became 2, in about one full run in ten).
#[cfg(test)]
pub(super) static EPOCH_GATE: std::sync::RwLock<()> = std::sync::RwLock::new(());


pub(super) fn bump_vault_epoch() {
    #[cfg(test)]
    let _shared =
        (!HOLDS_EPOCH_GATE.with(std::cell::Cell::get)).then(|| EPOCH_GATE.read().unwrap_or_else(|e| e.into_inner()));
    VAULT_EPOCH.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
}

pub(super) fn set_auth_present(state: &AppState, present: bool) {
    if let Ok(mut guard) = state.auth_present.lock() {
        if *guard != present {
            UI_SESSION_REVISION.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        *guard = present;
    }
}

pub(super) fn set_auth_email(state: &AppState, email: Option<String>) {
    // Per-account (decision 0800): the signed-in address belongs to the active
    // account, not the process. No active account (registry not synthesized) is
    // a silent no-op, mirroring the old poisoned-mutex swallow.
    if let Ok(acct) = state.active_account()
        && let Ok(mut guard) = acct.auth_email.lock()
    {
        if *guard != email {
            UI_SESSION_REVISION.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        *guard = email;
    }
}

/// Starts the sync engine for the configured sync root, if there is one. It takes no keys: the session
/// is read under the engine slot by `spawn_bound_engine` (lead ruling T9-starts), so a Lock that got
/// there first leaves nothing to start (`Ok`, with the vault left locked).
pub(super) async fn start_engine_if_possible(
    app: tauri::AppHandle,
    state: &State<'_, AppState>,
    #[cfg(target_os = "windows")] _transition: &tokio::sync::MutexGuard<'_, ()>,
) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        windows_cf::ensure_reactivation_allowed()?;
        SESSION_COMMANDS.open()?;
    }
    #[cfg(target_os = "windows")]
    let generation = SESSION_COMMANDS.generation()?;
    if let Ok(cfg) = DesktopConfig::load() {
        let Some(root) = cfg.sync_root else { return Ok(()) };
        // The engine + pause flag are per-account (decision 0800). No active
        // account → nothing to start.
        let Ok(acct) = state.active_account() else {
            return Ok(());
        };
        // Rehydrate the persisted pause state before spawning so a
        // restart-while-paused stays paused (the in-memory AtomicBool
        // always defaults to false; desktop.toml is the source of truth).
        rehydrate_sync_paused(&acct.sync_paused, cfg.pause_sync);
        let mut engine_slot = acct.engine.lock().await;
        engine_start_refusal(&acct)?;
        if let Some(prev) = engine_slot.take() {
            // Task 1538 Codex P1: this is a re-login/sync-root-change
            // respawn, not sign-out — there's no purge to gate here, but an
            // unconfirmed stop is still worth knowing about (a not-really-
            // gone previous task could still be touching the same state.db
            // the freshly spawned runner is about to open).
            if !stop_engine_in_slot(&acct, prev).await.is_stopped() {
                #[cfg(target_os = "windows")]
                return Err("Could not stop the previous sync engine; retry locking before restarting sync.".into());
                tracing::warn!("previous engine did not confirm termination before respawning a new one");
            }
        }
        // Fix round 1: an unconfirmed stop of the old engine has set the flag; refuse the new one.
        engine_start_refusal(&acct)?;
        #[cfg(target_os = "windows")]
        {
            windows_cf::ensure_reactivation_allowed()?;
            SESSION_COMMANDS.validate_start(generation)?;
        }
        // A vault locked in the meantime is not an error here: nothing to start, and Lock has said so. A start
        // deferred for a Finder removal is not one either: the reconciler removes the domain and its next check
        // starts the engine.
        match spawn_bound_engine(app, state, &acct, &mut engine_slot, root)? {
            EngineStart::Started => {}
            EngineStart::NoSession => {
                tracing::info!("engine start skipped: the vault was locked before it held the engine slot");
            }
            EngineStart::FinderRemovalOwed => {
                tracing::info!("engine start deferred: the previous account's Finder domain is removed first");
            }
        }
    }
    Ok(())
}

// ── IPC commands: first-launch onboarding ────────────────────────────────────

/// Open the sign-in surface (`surfaces::policy::onboarding_surface`).
///
/// A Mac signs in in the Beebeeb window (task 1683 slice 2, two-surface spec §7.4): the signed-out window shows the
/// sign-in screens itself, so this only shows `main` on Home, and the `onboarding` window is never built there.
/// Windows keeps its wider two-column wizard (`WindowsFirstRun.tsx`, label `windows-onboarding`) and Linux the compact
/// `Onboarding.tsx` (label `onboarding`). Re-calling when the window already exists is a no-op (focus only).
pub(crate) fn open_onboarding_window_impl(app: &tauri::AppHandle) -> Result<(), String> {
    match surfaces::policy::onboarding_surface(surfaces::policy::Platform::current()) {
        surfaces::policy::OnboardingSurface::BeebeebWindow => show_sign_in_in_beebeeb_window(app),
        #[cfg(not(target_os = "macos"))]
        surfaces::policy::OnboardingSurface::Window(label) => open_onboarding_window_labelled(app, label),
        #[cfg(target_os = "macos")]
        surfaces::policy::OnboardingSurface::Window(_) => Err("A Mac signs in in the Beebeeb window.".to_string()),
    }
}

/// A Mac's sign-in: the Beebeeb window on Home, which shows sign-in by itself when there is no account, or "Sign in
/// again" for an ended one. Nothing is cleared first; the window decides what to show from the account it reads.
#[cfg(target_os = "macos")]
pub(super) fn show_sign_in_in_beebeeb_window(app: &tauri::AppHandle) -> Result<(), String> {
    show_main_window(app, MainNav::Home);
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub(super) fn show_sign_in_in_beebeeb_window(_app: &tauri::AppHandle) -> Result<(), String> {
    Err("Only the macOS app signs in in the Beebeeb window.".to_string())
}

/// R8: “Sign in again” on a Mac. Unlike the sign-in above, which lets the window decide from the account it reads, this
/// asks the window for the card: a click on “Sign in again” always lands on it, even when the session was marked ended and
/// has since recovered (review P3-1). Nothing is cleared first.
#[cfg(target_os = "macos")]
pub(super) fn show_reauth_in_beebeeb_window(app: &tauri::AppHandle) -> Result<(), String> {
    show_main_window(app, MainNav::Reauth);
    Ok(())
}

/// The first-run window of Windows and Linux. It is not compiled on a Mac, which never builds it.
///
/// The window is wide enough for the two-column shell and centred. It mounts `Onboarding.tsx` (`?window=onboarding`)
/// on Linux or the Windows wizard (`platform=windows`) selected by the label, matching the `windows-onboarding` entry
/// in tauri.conf.json.
#[cfg(not(target_os = "macos"))]
pub(super) fn open_onboarding_window_labelled(app: &tauri::AppHandle, label: &'static str) -> Result<(), String> {
    let (url, width, height) = if label == "windows-onboarding" {
        ("index.html?window=onboarding&platform=windows", 860.0_f64, 640.0_f64)
    } else {
        ("index.html?window=onboarding", 860.0_f64, 640.0_f64)
    };

    if let Some(existing) = app.get_webview_window(label) {
        let _ = existing.show();
        let _ = existing.set_focus();
        return Ok(());
    }

    let win = tauri::WebviewWindowBuilder::new(app, label, tauri::WebviewUrl::App(url.into()))
        .title("Welcome to Beebeeb")
        .inner_size(width, height)
        .min_inner_size(780.0, 560.0)
        .resizable(true)
        .center()
        .build()
        .map_err(|e| e.to_string())?;
    // Explicit show + focus: tauri.conf.json declares the window
    // `visible: false`; without these calls the window is created but
    // never appears on screen.
    let _ = win.show();
    let _ = win.set_focus();

    Ok(())
}

/// IPC wrapper so the frontend can also open the sign-in surface
/// (e.g., from Account page → "Switch account").
#[tauri::command]
pub(super) async fn open_onboarding_window(app: tauri::AppHandle) -> Result<(), String> {
    open_onboarding_window_impl(&app)
}

/// R8: "Sign in again" opens sign-in in place. Nothing is cleared first. On a Mac that is the Beebeeb window, which
/// shows "Sign in again" for an ended account (two-surface spec §7.4); on Linux the onboarding window starts at the
/// sign-in step whatever `sync_status` says (`mode=reauth`); Windows keeps its own flow.
#[tauri::command]
pub(super) async fn open_reauth_window(app: tauri::AppHandle) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let _ = app;
        Err("Only available on macOS and Linux.".to_string())
    }
    #[cfg(target_os = "macos")]
    {
        show_reauth_in_beebeeb_window(&app)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        const URL: &str = "index.html?window=onboarding&mode=reauth";
        if let Some(existing) = app.get_webview_window("onboarding") {
            existing
                .eval("window.location.search = '?window=onboarding&mode=reauth'")
                .map_err(|e| e.to_string())?;
            let _ = existing.show();
            let _ = existing.set_focus();
            return Ok(());
        }
        let window = tauri::WebviewWindowBuilder::new(&app, "onboarding", tauri::WebviewUrl::App(URL.into()))
            .title("Welcome to Beebeeb")
            .inner_size(860.0, 640.0)
            .min_inner_size(780.0, 560.0)
            .resizable(true)
            .center()
            .build()
            .map_err(|e| e.to_string())?;
        let _ = window.show();
        let _ = window.set_focus();
        Ok(())
    }
}

/// The sign-in card's "Quit Beebeeb" (spec C §7.1): the same exit as the menu's Quit and ⌘Q. It carries an exit code,
/// so `keep_running_on_exit_request` does not turn it down.
#[tauri::command]
pub(super) fn quit_beebeeb(app: tauri::AppHandle) {
    app.exit(0)
}

/// Result of an OPAQUE sign-in attempt handed back to the frontend.
///
/// `requires_2fa == false` means the session is fully installed and onboarding
/// can proceed to the recovery-phrase step. `requires_2fa == true` means the
/// password was accepted but a TOTP code is still needed: `desktop_login`
/// stashed a [`Pending2fa`] in `AppState`, and the frontend must collect the
/// 6-digit code and call `desktop_login_2fa` to complete sign-in.
///
/// R8 adds `reauthenticated` (the account already on this Mac signed in again in place; with `vault_unlocked` its
/// keys are in memory, so no recovery phrase) and `account_mismatch` (another account: nothing changed on this Mac,
/// and the frontend shows the warning with the number of changes that have not uploaded). `key_replaced` is true only
/// for a re-sign-in whose kept vault key the server no longer accepts (spec §5.6): the key was removed, so the recovery
/// phrase step follows and says why. All five fields are ALWAYS serialized
/// (`{"requires_2fa":false,"reauthenticated":false,"vault_unlocked":false,"key_replaced":false,"account_mismatch":null}`
/// for a plain sign-in): the frontend's `settledFrom` reads them strictly, and
/// `login_outcome_json_is_the_frontends_contract` pins every shape.
#[derive(serde::Serialize)]
pub(super) struct LoginOutcome {
    pub(super) requires_2fa: bool,
    pub(super) reauthenticated: bool,
    pub(super) vault_unlocked: bool,
    pub(super) key_replaced: bool,
    pub(super) account_mismatch: Option<AccountMismatchDto>,
}

/// What the frontend needs to warn before an account switch: how many changes on this Mac have not uploaded yet.
/// On a Mac (spec 2026-10-10 W11) also the ask snapshot the switch step renders (`unsent`), or, when the count could
/// not be read, the coded E3 the step shows as a line (`unsent_error`); each is left out when it is `None`.
#[derive(serde::Serialize)]
pub(super) struct AccountMismatchDto {
    pub(super) pending_changes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) unsent: Option<sign_out::snapshot::AskCounts>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) unsent_error: Option<String>,
}

impl LoginOutcome {
    pub(super) fn signed_in() -> Self {
        Self {
            requires_2fa: false,
            reauthenticated: false,
            vault_unlocked: false,
            key_replaced: false,
            account_mismatch: None,
        }
    }

    pub(super) fn needs_2fa() -> Self {
        Self {
            requires_2fa: true,
            ..Self::signed_in()
        }
    }

    /// A browser sign-in that installed its session: the handoff carried the vault key, so the vault is unlocked (spec C
    /// C-S1). The password paths cannot say that: they sign in without a key and the recovery phrase follows.
    pub(super) fn browser_signed_in() -> Self {
        Self {
            vault_unlocked: true,
            ..Self::signed_in()
        }
    }

    #[cfg_attr(target_os = "windows", allow(dead_code))]
    pub(super) fn reauthenticated(vault_unlocked: bool, key_replaced: bool) -> Self {
        Self {
            reauthenticated: true,
            vault_unlocked,
            key_replaced,
            ..Self::signed_in()
        }
    }

    #[cfg_attr(target_os = "windows", allow(dead_code))]
    pub(super) fn account_mismatch(
        pending_changes: u64,
        unsent: Option<sign_out::snapshot::AskCounts>,
        unsent_error: Option<String>,
    ) -> Self {
        Self {
            account_mismatch: Some(AccountMismatchDto {
                pending_changes,
                unsent,
                unsent_error,
            }),
            ..Self::signed_in()
        }
    }
}

/// Authenticate with the current OPAQUE login endpoints.
///
/// This intentionally mirrors web and iOS: sign-in proves account access with
/// email/password only. If this Mac does not already have the vault key in
/// Keychain, onboarding moves to the separate recovery-phrase unlock step.
///
/// When the account has 2FA enabled, the OPAQUE finish returns
/// `requires_2fa: true` plus a short-lived `partial_token`. Rather than
/// erroring, we stash that token (with the email) in `AppState::pending_2fa`
/// and return `LoginOutcome { requires_2fa: true }`; the frontend then calls
/// `desktop_login_2fa` with the user's TOTP code to mint the real session.
///
/// Returns `Err` on HTTP errors, network failures, or OPAQUE failures.
///
/// Spec: docs/superpowers/plans/2026-05-07-desktop-sync-client.md (onboarding §1)
#[tauri::command]
pub(super) async fn desktop_login(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    email: String,
    password: String,
) -> Result<LoginOutcome, String> {
    #[cfg(target_os = "windows")]
    let attempt = AUTH_ATTEMPTS.begin()?;
    let work = async {
        // This sign-in is one session transition (lead ruling 1, Task 12): captured before it looks at anything, so a
        // Lock, a Sign-out or another sign-in that completes while it waits for the server is never followed by its
        // writes.
        let mut turn = state.active_account()?.session_generation();
        // Windows refuses a sign-in while a session exists and never asks which account it is (R8).
        #[cfg(target_os = "windows")]
        let _ = &app;
        #[cfg(target_os = "windows")]
        let _transition = SESSION_TRANSITION.lock().await;
        #[cfg(target_os = "windows")]
        AUTH_ATTEMPTS.validate(&attempt)?;
        #[cfg(target_os = "windows")]
        {
            SESSION_COMMANDS.ensure_reactivation_allowed()?;
            windows_cf::ensure_reactivation_allowed()?;
        }
        #[cfg(target_os = "windows")]
        if state.auth_present.lock().map(|present| *present).unwrap_or(true) {
            return Err("Sign out of the current account before signing in again.".into());
        }

        #[cfg(target_os = "windows")]
        drop(_transition);
        let base_url = runner::api_base_url();
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .default_headers(api_client::provenance_headers())
            .build()
            .map_err(|e| format!("reqwest build: {e}"))?;

        let email = account_binding::canonical_email(&email);
        if email.is_empty() || password.is_empty() {
            return Err("Email and password are required.".to_string());
        }

        let login_start = beebeeb_core::opaque_protocol::client_login_start(password.as_bytes())
            .map_err(|e| format!("opaque login start: {e}"))?;
        let client_message = encode_base64(&login_start.message);

        let start_resp = client
            .post(format!("{base_url}/api/v1/opaque/login-start"))
            .json(&serde_json::json!({ "email": email, "client_message": client_message }))
            .send()
            .await
            .map_err(|e| format!("network error: {e}"))?;
        if start_resp.status() == reqwest::StatusCode::UNAUTHORIZED
            || start_resp.status() == reqwest::StatusCode::NOT_FOUND
        {
            return Err("Invalid email or password".to_string());
        }
        if !start_resp.status().is_success() {
            let status = start_resp.status();
            let body = start_resp.text().await.unwrap_or_default();
            return Err(format!("Login start failed ({status}): {body}"));
        }

        let start_body: serde_json::Value = start_resp
            .json()
            .await
            .map_err(|e| format!("parse login start response: {e}"))?;
        let server_message = start_body
            .get("server_message")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "No server_message in login start response".to_string())?;
        let server_state = start_body
            .get("server_state")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "No server_state in login start response".to_string())?;
        // Dual-KSF login (task 0815): the server forwards the account's OPAQUE KSF
        // version (0 = legacy Identity, 1 = Argon2id) in the login-start response.
        // The client must finish under the matching CipherSuite, so thread it into
        // client_login_finish — mirrors web/iOS and the server's own call shape in
        // beebeeb-api/tests/api_integration.rs (`as u32` from the JSON number).
        let ksf_version = start_body
            .get("ksf_version")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| "No ksf_version in login start response".to_string())? as u32;
        let server_message_bytes =
            decode_base64(server_message).map_err(|e| format!("invalid OPAQUE server message: {e}"))?;
        let login_finish = beebeeb_core::opaque_protocol::client_login_finish(
            &login_start.state,
            password.as_bytes(),
            &server_message_bytes,
            ksf_version,
        )
        .map_err(|e| format!("Invalid email or password ({e})"))?;

        let finish_resp = client
            .post(format!("{base_url}/api/v1/opaque/login-finish"))
            .json(&serde_json::json!({
                "email": email,
                "client_message": encode_base64(&login_finish.message),
                "server_state": server_state,
            }))
            .send()
            .await
            .map_err(|e| format!("network error: {e}"))?;
        if finish_resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err("Invalid email or password".to_string());
        }
        if !finish_resp.status().is_success() {
            let status = finish_resp.status();
            let body = finish_resp.text().await.unwrap_or_default();
            return Err(format!("Login finish failed ({status}): {body}"));
        }
        let finish_body: serde_json::Value = finish_resp
            .json()
            .await
            .map_err(|e| format!("parse login finish response: {e}"))?;
        if finish_body
            .get("requires_2fa")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            // Password proven, but the account has 2FA. Capture the short-lived
            // partial session token (+ email) so `desktop_login_2fa` can complete
            // the sign-in once the user enters their TOTP code. The partial token is
            // server-validated and short-lived — never log it.
            #[cfg(target_os = "windows")]
            let _transition = SESSION_TRANSITION.lock().await;
            #[cfg(target_os = "windows")]
            AUTH_ATTEMPTS.validate(&attempt)?;
            let partial_token = finish_body
                .get("partial_token")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "No partial_token in login finish response".to_string())?
                .to_string();
            // A Lock or a Sign-out that completed while the password was checked clears the challenge; this one is not
            // set after it. macOS and Linux only: Windows orders this step with `SESSION_TRANSITION` and its attempt
            // fence, as before (Task 12 fix round 1, item 3).
            #[cfg(not(target_os = "windows"))]
            {
                let acct = state.active_account()?;
                if !acct.session_unchanged_since(turn) {
                    return Err(SessionChanged::of(&acct).into());
                }
            }
            {
                let mut guard = state
                    .pending_2fa
                    .lock()
                    .map_err(|_| "pending 2FA mutex poisoned".to_string())?;
                *guard = Some(Pending2fa {
                    partial_token,
                    email: email.clone(),
                });
            }
            tracing::info!("desktop login requires 2FA; awaiting TOTP code");
            return Ok(LoginOutcome::needs_2fa());
        }
        // Wiped when this login is done with it (it lives only as long as the sign-in handshake).
        let session_token = zeroize::Zeroizing::new(
            finish_body
                .get("session_token")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "No session_token in login finish response".to_string())?
                .to_string(),
        );

        let profile = fetch_session_profile(&client, &base_url, &session_token).await?;
        #[cfg(target_os = "windows")]
        let _transition = SESSION_TRANSITION.lock().await;
        #[cfg(target_os = "windows")]
        AUTH_ATTEMPTS.validate(&attempt)?;
        #[cfg(target_os = "windows")]
        if state.auth_present.lock().map(|present| *present).unwrap_or(true) {
            return Err("Sign out of the current account before signing in again.".into());
        }

        // R8 (spec 2026-10-06): the account already on this Mac signs in again in place; another account is an
        // account switch. Decided before anything is stored.
        #[cfg(not(target_os = "windows"))]
        {
            let settlement = match settle_sign_in(app.clone(), &state, &session_token, &profile, None, &mut turn).await
            {
                Ok(settlement) => settlement,
                Err(failure) => {
                    // A re-sign-in that failed before its token reached the Keychain stored nothing of the new
                    // session, so it is revoked, as a first sign-in whose token could not be stored is.
                    if !failure.token_stored {
                        let _ = revoke_desktop_session(&client, &base_url, &session_token).await;
                    }
                    return Err(failure.message);
                }
            };
            match settlement {
                SignInSettlement::Fresh => {}
                SignInSettlement::Reauthenticated {
                    vault_unlocked,
                    key_replaced,
                } => {
                    return Ok(LoginOutcome::reauthenticated(vault_unlocked, key_replaced));
                }
                SignInSettlement::AccountMismatch {
                    pending_changes,
                    unsent,
                    unsent_error,
                } => {
                    // Nothing on this Mac changed. The session just minted for the other account is revoked;
                    // after the switch the person signs in again.
                    let _ = revoke_desktop_session(&client, &base_url, &session_token).await;
                    return Ok(LoginOutcome::account_mismatch(pending_changes, unsent, unsent_error));
                }
                SignInSettlement::Unconfirmed => {
                    // The server could not say whose key this Mac holds: nothing changed, the new session is
                    // revoked, and trying again is safe.
                    let _ = revoke_desktop_session(&client, &base_url, &session_token).await;
                    return Err(SIGN_IN_ACCOUNT_UNKNOWN.to_string());
                }
            }
        }

        // Resolve the active account up front so we can both cache the profile and
        // segment the keychain write under its id (task 0800). The cache is
        // per-account (decision 0800); `pending_2fa` above stays on `AppState`.
        let acct = state.active_account()?;
        // The session is known by the server's spelling of the email, not by what was typed (R2).
        let email = session_email(Some(&profile)).unwrap_or(email);
        let stored = store_first_sign_in(&state, &acct, &mut turn, &session_token, &email, profile);
        if let Err(e) = stored {
            #[cfg(target_os = "windows")]
            drop(_transition);
            let _ = revoke_desktop_session(&client, &base_url, &session_token).await;
            return Err(e);
        }
        tracing::info!("desktop account session installed");

        Ok(LoginOutcome::signed_in())
    };
    #[cfg(target_os = "windows")]
    {
        attempt.run(work).await
    }
    #[cfg(not(target_os = "windows"))]
    {
        work.await
    }
}

/// A first sign-in by password (with or without a second factor): its token, the profile and the flags, written in the
/// sign-in's turn (lead ruling 1, Task 12). Refused, with nothing written, when any session transition happened since
/// the sign-in began (a Lock, a Sign-out, another sign-in that stored first); the caller then revokes the new session.
pub(super) fn store_first_sign_in(
    state: &AppState,
    acct: &AccountRuntime,
    turn: &mut account::SessionGeneration,
    token: &str,
    email: &str,
    profile: account_dto::AccountProfile,
) -> Result<(), String> {
    let write = claim_session_write(acct, turn)?;
    persist_session_token_to_keychain(&write, acct.id.as_str(), token, Some(email))?;
    if let Ok(mut guard) = acct.cached_profile.lock() {
        *guard = Some(profile);
    }
    set_auth_present(state, true);
    set_auth_email(state, Some(email.to_string()));
    Ok(())
}

/// Complete a 2FA-gated sign-in: trade the held partial token + the user's TOTP
/// code for a real session, then run the same post-login setup as the no-2FA
/// path in `desktop_login`.
///
/// Requires a prior `desktop_login` call that returned
/// `requires_2fa: true` (which stashed a [`Pending2fa`] in `AppState`). On an
/// invalid/expired code the server replies 401 → we surface a clear, retryable
/// error and DELIBERATELY keep the pending state so the user can retry within
/// the partial token's ~5-minute window. On success we clear the pending state.
///
/// Mirrors the web client's `verify2fa(partialToken, code)`:
/// `POST /api/v1/auth/2fa/verify` with body `{ partial_token, code }` →
/// `{ user_id, session_token }`. The TOTP code is never logged.
#[tauri::command]
pub(super) async fn desktop_login_2fa(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    code: String,
) -> Result<LoginOutcome, String> {
    #[cfg(target_os = "windows")]
    let attempt = AUTH_ATTEMPTS.begin()?;
    let work = async {
        // This sign-in is one session transition (lead ruling 1, Task 12): captured before it looks at anything, so a
        // Lock, a Sign-out or another sign-in that completes while it waits for the server is never followed by its
        // writes.
        let mut turn = state.active_account()?.session_generation();
        // Windows refuses a sign-in while a session exists and never asks which account it is (R8).
        #[cfg(target_os = "windows")]
        let _ = &app;
        #[cfg(target_os = "windows")]
        let _transition = SESSION_TRANSITION.lock().await;
        #[cfg(target_os = "windows")]
        AUTH_ATTEMPTS.validate(&attempt)?;
        #[cfg(target_os = "windows")]
        {
            SESSION_COMMANDS.ensure_reactivation_allowed()?;
            windows_cf::ensure_reactivation_allowed()?;
        }
        #[cfg(target_os = "windows")]
        drop(_transition);
        let base_url = runner::api_base_url();
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .default_headers(api_client::provenance_headers())
            .build()
            .map_err(|e| format!("reqwest build: {e}"))?;

        // Read (don't yet consume) the pending challenge: we only clear it on
        // success so an invalid code stays retryable within the 5-minute window.
        let (partial_token, email) = {
            let guard = state
                .pending_2fa
                .lock()
                .map_err(|_| "pending 2FA mutex poisoned".to_string())?;
            let pending = guard
                .as_ref()
                .ok_or_else(|| "No pending two-factor sign-in. Start sign-in again.".to_string())?;
            (pending.partial_token.clone(), pending.email.clone())
        };

        let code = code.trim().to_string();
        if code.is_empty() {
            return Err("Enter your authentication code.".to_string());
        }

        let verify_resp = client
            .post(format!("{base_url}/api/v1/auth/2fa/verify"))
            .json(&serde_json::json!({ "partial_token": partial_token, "code": code }))
            .send()
            .await
            .map_err(|e| format!("network error: {e}"))?;
        if verify_resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            // Invalid/expired code. Keep `pending_2fa` so the user can retry while
            // the partial token is still valid.
            return Err("Invalid authentication code".to_string());
        }
        if !verify_resp.status().is_success() {
            let status = verify_resp.status();
            let body = verify_resp.text().await.unwrap_or_default();
            return Err(format!("Two-factor verification failed ({status}): {body}"));
        }
        let verify_body: serde_json::Value = verify_resp
            .json()
            .await
            .map_err(|e| format!("parse 2FA verify response: {e}"))?;
        // Wiped when this login is done with it (it lives only as long as the sign-in handshake).
        let session_token = zeroize::Zeroizing::new(
            verify_body
                .get("session_token")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "No session_token in 2FA verify response".to_string())?
                .to_string(),
        );

        // Same post-finish setup as `desktop_login`'s no-2FA success path. The
        // cache is per-account (decision 0800); `pending_2fa` stays on `AppState`.
        let profile = fetch_session_profile(&client, &base_url, &session_token).await?;
        #[cfg(target_os = "windows")]
        let _transition = SESSION_TRANSITION.lock().await;
        #[cfg(target_os = "windows")]
        AUTH_ATTEMPTS.validate(&attempt)?;
        #[cfg(target_os = "windows")]
        if state.auth_present.lock().map(|present| *present).unwrap_or(true) {
            return Err("Sign out of the current account before signing in again.".into());
        }

        // R8 (spec 2026-10-06): the same check as `desktop_login`, before anything is stored. Whatever it finds, the
        // second factor is spent, so the held challenge is dropped on every way out but a first sign-in (which drops it
        // at the end).
        #[cfg(not(target_os = "windows"))]
        {
            let settlement = match settle_sign_in(app.clone(), &state, &session_token, &profile, None, &mut turn).await
            {
                Ok(settlement) => settlement,
                Err(failure) => {
                    if !failure.token_stored {
                        let _ = revoke_desktop_session(&client, &base_url, &session_token).await;
                    }
                    if let Ok(mut guard) = state.pending_2fa.lock() {
                        *guard = None;
                    }
                    return Err(failure.message);
                }
            };
            match settlement {
                SignInSettlement::Fresh => {}
                SignInSettlement::Reauthenticated {
                    vault_unlocked,
                    key_replaced,
                } => {
                    if let Ok(mut guard) = state.pending_2fa.lock() {
                        *guard = None;
                    }
                    return Ok(LoginOutcome::reauthenticated(vault_unlocked, key_replaced));
                }
                SignInSettlement::AccountMismatch {
                    pending_changes,
                    unsent,
                    unsent_error,
                } => {
                    // Nothing on this Mac changed. The session just minted for the other account is revoked.
                    let _ = revoke_desktop_session(&client, &base_url, &session_token).await;
                    if let Ok(mut guard) = state.pending_2fa.lock() {
                        *guard = None;
                    }
                    return Ok(LoginOutcome::account_mismatch(pending_changes, unsent, unsent_error));
                }
                SignInSettlement::Unconfirmed => {
                    let _ = revoke_desktop_session(&client, &base_url, &session_token).await;
                    if let Ok(mut guard) = state.pending_2fa.lock() {
                        *guard = None;
                    }
                    return Err(SIGN_IN_ACCOUNT_UNKNOWN.to_string());
                }
            }
        }

        let acct = state.active_account()?;
        // The session is known by the server's spelling of the email, not by what was typed (R2).
        let email = session_email(Some(&profile)).unwrap_or(email);
        let stored = store_first_sign_in(&state, &acct, &mut turn, &session_token, &email, profile);
        if let Err(e) = stored {
            #[cfg(target_os = "windows")]
            drop(_transition);
            let _ = revoke_desktop_session(&client, &base_url, &session_token).await;
            return Err(e);
        }
        // Challenge satisfied — drop the pending state so a stale partial token
        // can't be reused.
        if let Ok(mut guard) = state.pending_2fa.lock() {
            *guard = None;
        }
        tracing::info!("desktop account session installed after 2FA");

        Ok(LoginOutcome::signed_in())
    };
    #[cfg(target_os = "windows")]
    {
        attempt.run(work).await
    }
    #[cfg(not(target_os = "windows"))]
    {
        work.await
    }
}

/// Provision this Mac with the user's vault key after account sign-in.
///
/// A recovery phrase is only needed when the Mac does not already have a
/// Keychain-stored vault key. The account session was established by
/// `desktop_login`; this command derives the local vault key with
/// `beebeeb-core`, stores it in Keychain, and starts the sync runner.
#[tauri::command]
pub(super) async fn desktop_unlock_with_recovery_phrase(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    recovery_phrase: String,
) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    let attempt = AUTH_ATTEMPTS.begin()?;
    let work = async {
        #[cfg(target_os = "windows")]
        let _transition = SESSION_TRANSITION.lock().await;
        #[cfg(target_os = "windows")]
        AUTH_ATTEMPTS.validate(&attempt)?;
        #[cfg(target_os = "windows")]
        {
            SESSION_COMMANDS.ensure_reactivation_allowed()?;
            windows_cf::ensure_reactivation_allowed()?;
        }
        let acct = state.active_account()?;
        // The unlock is one session transition (lead ruling 1, Task 12): captured before it reads anything.
        let mut turn = acct.session_generation();
        let session_installed = acct
            .session
            .lock()
            .map_err(|_| "session mutex poisoned".to_string())?
            .is_some();
        if session_installed {
            let started = start_engine_if_possible(
                app,
                &state,
                #[cfg(target_os = "windows")]
                &_transition,
            )
            .await;
            keys_arrived(&state, KeysFrom::SignIn);
            return started;
        }

        let account_id = acct.id.as_str().to_string();
        let token = zeroize::Zeroizing::new(
            load_session_token_from_keychain(&account_id)?
                .ok_or_else(|| "Sign in before unlocking the vault.".to_string())?,
        );
        let email = acct.auth_email.lock().ok().and_then(|guard| guard.clone());
        let base_url = runner::api_base_url();
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .default_headers(api_client::provenance_headers())
            .build()
            .map_err(|e| format!("reqwest build: {e}"))?;
        // Verification is cancellable network work. Teardown must be able to
        // acquire the transition and revoke this attempt while it is pending.
        #[cfg(target_os = "windows")]
        drop(_transition);
        let verified_key = verify_vault_key_from_phrase(&client, &base_url, &token, &recovery_phrase).await?;
        #[cfg(target_os = "windows")]
        let _transition = SESSION_TRANSITION.lock().await;
        #[cfg(target_os = "windows")]
        AUTH_ATTEMPTS.validate(&attempt)?;
        install_recovered_session(&state, &acct, &mut turn, &token, email, &verified_key)?;
        // Both copies are wiped here, not at the end of the function: the engine start below awaits, and the
        // session in memory is the only holder of the keys from now on.
        drop(token);
        drop(verified_key);
        tracing::info!("vault provisioned from recovery phrase");
        let started = start_engine_if_possible(
            app,
            &state,
            #[cfg(target_os = "windows")]
            &_transition,
        )
        .await;
        keys_arrived(&state, KeysFrom::SignIn);
        started
    };
    #[cfg(target_os = "windows")]
    {
        attempt.run(work).await
    }
    #[cfg(not(target_os = "windows"))]
    {
        work.await
    }
}

/// The recovery-phrase unlock's writes, after the server confirmed the phrase: the vault key and the email are written,
/// and the session put in memory, in ONE turn of the unlock (lead ruling 1, Task 12), and only while the account it
/// started with is still the one stored (a sign-in, a Lock or a Sign-out may have run while the phrase was being
/// checked). Nothing can land between the write and the install.
pub(super) fn install_recovered_session(
    state: &AppState,
    acct: &AccountRuntime,
    turn: &mut account::SessionGeneration,
    token: &str,
    email: Option<String>,
    verified_key: &[u8; 32],
) -> Result<(), String> {
    {
        let write = store_recovered_vault_key(acct, turn, token, email.as_deref(), verified_key)?;
        let mut guard = acct.session.lock().map_err(|_| "session mutex poisoned".to_string())?;
        *guard = Some(Session {
            token: token.to_string(),
            master_key: *verified_key,
            email,
        });
        drop(guard);
        set_auth_present(state, true);
        // Ruling F9: the vault is unlocked now (after a replaced key too), so sync resumes; on every OS since spec D11.
        ask_to_resume_auth_paused(&write, acct);
        drop(write);
    }
    bump_vault_epoch();
    Ok(())
}

pub(super) const INCORRECT_RECOVERY_PHRASE: &str = "Incorrect recovery phrase. Check your words and try again.";

/// What the recovery-phrase unlock says when a Sign-out, or another account's sign-in, happened while it waited for the
/// server (Task 12 fix round 2, item 5).
pub(super) const UNLOCK_ACCOUNT_CHANGED: &str =
    "You were signed out, or another account signed in, while Beebeeb was unlocking. Try again.";

/// The recovery-phrase unlock's write of the verified vault key (and the account email), for the session it started
/// with: `started_token` is the token it read from the Keychain, `started_email` the identity it knew. Between that
/// read and this write the unlock waited for the server, and a sign-in or sign-out can have run: another account's
/// token may be stored now, or none, or the same account's replacement. In the unlock's turn (`turn`, see
/// [`claim_session_write`]) any session transition since it began stops the write, and the stored token, the identity
/// and the empty memory are checked again: one account's key is never written next to another account's token or
/// email. Returns the turn, still held, so the caller installs the session in memory before anything else can run.
pub(super) fn store_recovered_vault_key(
    acct: &AccountRuntime,
    turn: &mut account::SessionGeneration,
    started_token: &str,
    started_email: Option<&str>,
    master_key: &[u8; 32],
) -> Result<SessionWrite, String> {
    let account_id = acct.id.as_str();
    // The unlock's turn (lead ruling 1, Task 12): any session transition since the unlock began stops it here. Then,
    // in the same turn, the stored token, the identity and the empty memory are checked against what it started with.
    let write = claim_session_write(acct, turn).map_err(|changed| changed.recovery_unlock_sentence().to_string())?;
    let stored = load_session_token_from_keychain(account_id)?.map(zeroize::Zeroizing::new);
    let session_installed = acct
        .session
        .lock()
        .map_err(|_| "session mutex poisoned".to_string())?
        .is_some();
    let identity = acct
        .auth_email
        .lock()
        .map_err(|_| "auth email mutex poisoned".to_string())?
        .clone();
    if stored.as_ref().map(|token| token.as_str()) != Some(started_token)
        || session_installed
        || identity.as_deref() != started_email
    {
        return Err(UNLOCK_ACCOUNT_CHANGED.to_string());
    }
    persist_vault_key_to_keychain(&write, account_id, master_key)?;
    // `desktop_login` already persisted the email when it stored the token, but persist again here (idempotent) so
    // the invariant "a fully-provisioned session has its email in the store" holds even if memory and store drift.
    if let Some(email) = started_email {
        let vault = AuthVault::new(platform_keychain_store_for(account_id));
        if let Err(e) = vault.store_account_email(email) {
            // Non-fatal: the email is display metadata, not required to unlock.
            tracing::warn!(error = %e, "could not persist account email during recovery-phrase unlock");
        }
    }
    // The caller puts the session in memory before it lets this turn go.
    Ok(write)
}

/// Derive the vault key from a recovery phrase, prove it belongs to the
/// signed-in account, and only then hand it to `persist`.
///
/// `recover_from_phrase` derives *a* key from ANY checksum-valid BIP39 phrase —
/// it does not prove the phrase is this account's. Persisting an unverified key
/// keys the sync engine with a foreign key, so every upload from this device is
/// undecryptable on every other device (flow 7 P0). Same gate as web's
/// device-provision (`recoveredKeyMatchesAccount`, task 0874).
#[cfg(test)]
pub(super) async fn provision_vault_key_from_phrase<P>(
    client: &reqwest::Client,
    base_url: &str,
    session_token: &str,
    recovery_phrase: &str,
    persist: P,
) -> Result<[u8; 32], String>
where
    P: FnOnce([u8; 32]) -> Result<(), String>,
{
    let master_key = verify_vault_key_from_phrase(client, base_url, session_token, recovery_phrase).await?;
    persist(*master_key)?;
    Ok(*master_key)
}

/// Verification has no persistent side effects; callers fence installation.
pub(super) async fn verify_vault_key_from_phrase(
    client: &reqwest::Client,
    base_url: &str,
    session_token: &str,
    recovery_phrase: &str,
) -> Result<zeroize::Zeroizing<[u8; 32]>, String> {
    let recovery_phrase = normalize_recovery_phrase_input(recovery_phrase)?;
    let master_key_struct = beebeeb_core::recovery::recover_from_phrase(&recovery_phrase)
        .map_err(|_| "Recovery phrase does not match a valid 12-word Beebeeb phrase.".to_string())?;
    if !recovered_key_matches_account(client, base_url, session_token, &master_key_struct).await? {
        // `master_key_struct` zeroizes on drop.
        return Err(INCORRECT_RECOVERY_PHRASE.to_string());
    }
    Ok(zeroize::Zeroizing::new(master_key_struct.to_bytes()))
}
