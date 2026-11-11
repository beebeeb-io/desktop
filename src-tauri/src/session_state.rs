//! `session_state` items split out of `lib.rs`.

use super::*;

/// Task 2447: whose vault key is stored next to whose session token (declared here: lib.rs is not rustfmt-clean on v2).
#[path = "session_owner.rs"]
mod session_owner;
pub(crate) use session_owner::*;

/// Work a command starts as a task of its own, which may outlive the command (the popover's storage fetch), is bound to
/// the session when it is created: on Windows a lock cancels it and waits for what it holds to drop, and once the
/// session is closed it never starts (#139 review P1; tests/windows_session_wiring.rs guards the callers).
pub(super) fn session_bound<T>(
    work: impl std::future::Future<Output = Result<T, String>> + Send + 'static,
) -> impl std::future::Future<Output = Result<T, String>> + Send + 'static
where
    T: Send + 'static,
{
    #[cfg(target_os = "windows")]
    {
        let bound = SESSION_COMMANDS.bind(work);
        async move { bound?.await }
    }
    #[cfg(not(target_os = "windows"))]
    {
        work
    }
}

#[cfg(target_os = "windows")]
pub(super) async fn close_session_commands() -> Result<(), String> {
    let revoked = SESSION_COMMANDS.close();
    let auth_revoked = AUTH_ATTEMPTS.close();
    let worker = tokio::task::spawn_blocking(move || {
        if let Some(auth) = auth_revoked {
            auth.drain(std::time::Duration::from_secs(3))?;
        }
        SESSION_COMMANDS.drain(revoked, std::time::Duration::from_secs(3))
    });
    tokio::time::timeout(std::time::Duration::from_secs(4), worker)
        .await
        .map_err(|_| {
            "Vault lock failed: commands are still stopping. The vault is not locked. Retry locking.".to_string()
        })?
        .map_err(|e| format!("Vault lock failed: {e}"))?
}

// ── Session bridge (web ↔ rust) ───────────────────────────────────────────────

/// Authenticated session pushed in from the WebView after login.
///
/// `master_key` is the user's 32-byte vault root key, derived from the
/// OPAQUE export_key (or, for legacy accounts, from Argon2id over the
/// password). It is the input to per-file `derive_file_key()` calls
/// inside `beebeeb-sync` — the engine cannot do anything without it.
///
/// The struct is intentionally NOT `Clone`: there should be exactly one
/// authoritative copy of the master key in this process at any moment.
/// `take()` removes it from the registry on logout; everything else
/// reads it through a borrow under the `Mutex`.
///
/// `ZeroizeOnDrop` wipes the secret bytes when the `Session` is dropped — on
/// `lock_vault` / `clear_session` (both `take()` the `Option<Session>`, dropping
/// it) and on process teardown — so the master key and the session token never
/// linger in freed memory. All three fields are `Zeroize`: `[u8; 32]`,
/// `String` (token), and `Option<String>` (email) each implement it in
/// `zeroize` 1.x, so no field needs `#[zeroize(skip)]`.
#[derive(zeroize::ZeroizeOnDrop)]
pub struct Session {
    pub token: String,
    pub master_key: [u8; 32],
    pub email: Option<String>,
}

impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session")
            .field("token", &"<redacted>")
            .field("master_key", &"<redacted>")
            .field("email", &self.email.as_deref().map(|_| "<set>").unwrap_or("<unset>"))
            .finish()
    }
}

/// A login that proved the password (OPAQUE) but still needs a second factor.
///
/// Held in [`AppState::pending_2fa`] between `desktop_login` (which receives the
/// server's `requires_2fa` + `partial_token`) and `desktop_login_2fa` (which
/// carries the partial token plus the user's TOTP code to `/auth/2fa/verify` to
/// mint the real session). The `partial_token` is a server-issued, short-lived
/// (≈5-minute) partial session token — it is NOT key material and is validated
/// server-side, so it is never logged. `email` is the lowercased address from
/// `desktop_login`, reused for the same post-login setup the success path runs.
///
/// `ZeroizeOnDrop` wipes the partial token (a live, short-lived server
/// credential) from freed heap whenever `pending_2fa` is replaced or cleared —
/// mirroring how `Session` zeroizes its token. Both `String` fields implement
/// `Zeroize` in `zeroize` 1.x, so the derive applies without `#[zeroize(skip)]`.
pub(super) struct Pending2fa {
    /// Core's challenge (B10b): it holds the partial token in a wiped buffer, so dropping it wipes the token.
    pub(super) second_factor: beebeeb_core::auth_flow::sign_in::PendingSecondFactor,
    pub(super) email: String,
}

/// Tauri-managed shared state. Held behind a `Mutex` because the web
/// thread (IPC handlers) and the sync-engine task both need access.
/// Cheap to lock — the registry and these flags change rarely.
///
/// Phase 0 multi-account refactor (decision 0800): the per-account live
/// state (`session`, `engine`, `engine_state`, `sync_paused`,
/// `cached_profile`, `auth_email`) moved off `AppState` onto
/// [`AccountRuntime`]; handlers reach it through [`AppState::active_account`].
/// At N=1 this is invisible — exactly one account is synthesized at startup.
/// What stays on `AppState` is the genuinely process-global / login-in-flight
/// state: `auth_present` (a process-wide "is anyone signed in" seed) and
/// `pending_2fa` (a login challenge that has no account yet).
pub struct AppState {
    /// The account registry. Exactly one entry in Phase 0 (synthesized by
    /// `synthesize_single_account` at startup). `Arc<AccountRuntime>` so
    /// `active_account()` can hand out a shared handle that callers lock,
    /// without ever cloning the `!Clone` `Session` inside it.
    pub accounts: Mutex<Vec<Arc<AccountRuntime>>>,
    /// Id of the currently-active account. `active_account()` resolves this,
    /// falling back to the first registered account.
    pub active_account_id: Mutex<Option<AccountId>>,
    /// Process-wide "is a session present in the credential store" seed,
    /// set from `keychain_session_present()` at startup and on login/logout.
    /// Stays on `AppState` (not per-account) — it gates onboarding routing
    /// before any account is unlocked.
    pub auth_present: Mutex<bool>,
    /// A password-verified login awaiting its TOTP second factor. Set by
    /// `desktop_login` when the OPAQUE finish returns `requires_2fa: true`;
    /// consumed (and cleared on success) by `desktop_login_2fa`. `None`
    /// whenever no 2FA challenge is in flight. Stays on `AppState`: during the
    /// challenge there is no account yet to attach it to.
    pub(super) pending_2fa: Mutex<Option<Pending2fa>>,
    /// Last native-menu zoom scale applied to WebViews. Tauri exposes
    /// `set_zoom` but not a readback API, so the menu owns this small bit of
    /// process state.
    pub menu_zoom_scale: Mutex<f64>,
    /// The macOS Finder reconciler (spec 2026-10-06). Set once in `setup()` on macOS; empty on
    /// Windows/Linux and in tests that install none.
    pub finder_setup: std::sync::OnceLock<finder_setup::driver::FinderSetupHandle>,
    /// The sign-out flow's state (spec 2026-10-10 §5.1): the newest ask snapshot, which the gate compares with.
    pub(crate) sign_out: sign_out::SignOutState,
    /// The set-up step a password sign-in without a key left open, for the life of the process (review P2-3 on the Mac
    /// sign-in PR): see [`PendingSetup`].
    pub(super) pending_setup: Mutex<Option<HeldSetup>>,
    /// The account's storage summary for `popover_snapshot` (task 2179). Here, not on the popover's runtime, so
    /// sign-out clears it with the rest of the account (#139 review P2-3).
    pub(crate) popover_storage: Arc<Mutex<popover_data::StorageCache>>,
}

impl AppState {
    /// Resolve the active account runtime.
    ///
    /// Returns the account whose id matches `active_account_id`, falling back
    /// to the first registered account when the id is unset or stale. Returns
    /// `Err` only when the registry is empty (i.e. before
    /// `synthesize_single_account` has run) — at N=1 it always resolves after
    /// startup.
    ///
    /// Hands out an `Arc<AccountRuntime>`; callers lock the inner mutexes
    /// (`acct.session.lock()`, `acct.engine.lock().await`, …). There is
    /// deliberately NO `active_account_mut()`: interior mutability keeps the
    /// 1:1 locking discipline and never moves the `!Clone` `Session`.
    pub fn active_account(&self) -> Result<Arc<AccountRuntime>, String> {
        let accounts = self
            .accounts
            .lock()
            .map_err(|_| "accounts registry mutex poisoned".to_string())?;
        if accounts.is_empty() {
            return Err("no active account".to_string());
        }
        let active_id = self.active_account_id.lock().ok().and_then(|g| g.clone());
        if let Some(id) = active_id
            && let Some(found) = accounts.iter().find(|a| a.id == id)
        {
            return Ok(found.clone());
        }
        // Fallback: first registered account (the only one in Phase 0).
        Ok(accounts[0].clone())
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            accounts: Mutex::new(Vec::new()),
            active_account_id: Mutex::new(None),
            auth_present: Mutex::new(false),
            pending_2fa: Mutex::new(None),
            menu_zoom_scale: Mutex::new(1.0),
            finder_setup: std::sync::OnceLock::new(),
            sign_out: sign_out::SignOutState::default(),
            pending_setup: Mutex::new(None),
            popover_storage: Arc::new(Mutex::new(popover_data::StorageCache::default())),
        }
    }
}

/// The step a password sign-in on a Mac with no vault key leaves open: “Set up this Mac”, the recovery phrase. The Beebeeb
/// window keeps its sign-in flow in a module store, and ⌘W destroys the window and the store with it, while the session
/// stays and the key was never delivered; a window opened again then showed Home, whose locked card had nothing to unlock
/// from (review P2-3). The window tells Rust when it enters the step (`set_pending_setup`) and asks once when it opens
/// signed in and locked (`pending_setup`). Held for the life of the process: a relaunch is the Keychain signal of task
/// 1747's Part 2.
///
/// It is forgotten the moment keys arrive on this Mac ([`keys_arrived`]: every unlock and every sign-in that puts a session
/// in memory), and it is answered only while it is still true (an account signed in, its vault locked), so a sign-out or
/// any other way the account leaves makes it inert without touching the sign-out code; the next sign-in sets it again or
/// brings keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub(super) struct PendingSetup {
    /// The kept key was replaced (spec §5.6), so the card says why the phrase is asked.
    pub(super) key_replaced: bool,
}

/// A pending step and the account it was set for. The slot outlives a sign-out (the sign-out code is untouched), so a step
/// of account A would otherwise be answered for account B who signed in after A and is signed in and locked; it is
/// answered only for the account it was set for (review P3-4 of the delta review).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct HeldSetup {
    pub(super) step: PendingSetup,
    pub(super) account: account_binding::Identity,
}

pub(super) fn remember_pending_setup(
    slot: &Mutex<Option<HeldSetup>>,
    key_replaced: bool,
    account: account_binding::Identity,
) {
    *slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(HeldSetup {
        step: PendingSetup { key_replaced },
        account,
    });
}

pub(super) fn forget_pending_setup(slot: &Mutex<Option<HeldSetup>>) {
    *slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
}

/// The pending step, if it is still true: an account is signed in, its vault is locked, and it is the account the step was
/// set for (`account_binding::same_account`: an identity that cannot be compared resumes nothing). Reading it does not use
/// it up.
pub(super) fn resume_pending_setup(
    slot: &Mutex<Option<HeldSetup>>,
    signed_in: bool,
    vault_unlocked: bool,
    current: &account_binding::Identity,
) -> Option<PendingSetup> {
    if !signed_in || vault_unlocked {
        return None;
    }
    let held = slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone()?;
    (account_binding::same_account(&held.account, current) == Some(true)).then_some(held.step)
}

/// Who this Mac's credentials name, for a pending step: the cached profile's user id and email, else the session's or the
/// Keychain's email (`credentials_identity`). Windows has no Beebeeb window to resume, so it names nobody there.
pub(super) fn pending_setup_identity(acct: &AccountRuntime) -> account_binding::Identity {
    #[cfg(not(target_os = "windows"))]
    {
        credentials_identity(acct)
    }
    #[cfg(target_os = "windows")]
    {
        let _ = acct;
        account_binding::Identity::default()
    }
}

/// The window enters “Set up this Mac”: Rust keeps the step for the process, for the account whose credentials this Mac holds
/// (see [`PendingSetup`]).
#[tauri::command]
pub(super) fn set_pending_setup(state: State<'_, AppState>, key_replaced: bool) -> Result<(), String> {
    let acct = state.active_account()?;
    remember_pending_setup(&state.pending_setup, key_replaced, pending_setup_identity(&acct));
    Ok(())
}

/// A window that opens signed in and locked asks whether “Set up this Mac” is still owed. `logged_in` is `sync_status`'s own
/// reading: a session in memory, or a token in the credential store.
#[tauri::command]
pub(super) fn pending_setup(state: State<'_, AppState>) -> Result<Option<PendingSetup>, String> {
    let acct = state.active_account()?;
    let vault_unlocked = acct.session.lock().map(|guard| guard.is_some()).unwrap_or(false);
    let auth_present = state.auth_present.lock().map(|guard| *guard).unwrap_or(false);
    Ok(resume_pending_setup(
        &state.pending_setup,
        vault_unlocked || auth_present,
        vault_unlocked,
        &pending_setup_identity(&acct),
    ))
}

pub(super) fn keychain_error(context: &str, error: impl fmt::Display) -> String {
    format!("{context}: {error}")
}

/// Drop the cached `/auth/me` profile from the active account.
///
/// Called on logout (`clear_session`) and lock (`lock_vault`) so the
/// `account_profile` IPC can never cache-hit a stale (logged-out / locked)
/// identity. A subsequent login / unlock re-fetches and re-caches. Poisoned
/// mutex is swallowed — clearing must never fail a logout/lock from the
/// user's POV. Takes `&AppState` (not `State<'_, _>`) so it's unit-testable
/// against a synthesized `AppState`. A missing active account (registry not yet
/// synthesized) is also a silent no-op.
pub(super) fn clear_cached_profile(state: &AppState) {
    if let Ok(acct) = state.active_account()
        && let Ok(mut guard) = acct.cached_profile.lock()
    {
        *guard = None;
    }
}

/// One lock for everything that writes or clears a session (Task 11; Task 12 lead ruling 1): the Keychain items, the
/// session in memory with the flags that mirror it, and the owner record a session writes. Every session transition
/// takes its turn under it: [`claim_session_write`] checks, under this lock, that no other transition of the account
/// happened since the writer's own transition began, and moves the account's [`account::SessionGeneration`] on; Lock and
/// Sign-out move it on under it too ([`end_sessions_in_flight`], [`finish_sign_out_turn`]). Held only across synchronous
/// work, never across an `.await` (clippy's `await_holding_lock` checks it). While a turn writes `state.db` or the
/// Keychain, another transition that needs the lock waits for that I/O (up to the database's busy timeout). A poisoned
/// lock is recovered: it guards no data, only the order.
pub(super) static SESSION_KEYCHAIN_WRITES: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub(super) fn lock_session_keychain_writes() -> std::sync::MutexGuard<'static, ()> {
    SESSION_KEYCHAIN_WRITES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Proof that the holder has the session-write lock for a session transition that is still current. In production only
/// [`claim_session_write`], [`check_session_turn`], [`end_sessions_in_flight`] and [`finish_sign_out_turn`] make one,
/// and every writer of a session (the Keychain items, and the functions that install, replace or name the session in
/// memory) takes one, so none of them can write outside a checked turn. Dropping it ends the turn. The one clear that
/// takes the engine-slot guard instead, [`clear_session_holding_slot`] (Lock and Sign-out), ends every transition in
/// flight first and then clears outside this lock on purpose: `bump_vault_epoch` must never wait under it.
pub(super) struct SessionWrite {
    pub(super) _writes: std::sync::MutexGuard<'static, ()>,
}

impl fmt::Debug for SessionWrite {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SessionWrite")
    }
}

#[cfg(test)]
impl SessionWrite {
    /// Tests only: the lock without a generation check, to seed the in-memory Keychain the way an earlier sign-in left
    /// it. Used as a temporary, so the lock is released at the end of the statement.
    pub(super) fn for_test() -> Self {
        Self {
            _writes: lock_session_keychain_writes(),
        }
    }
}

/// What a sign-in says when another session transition (a Sign-out, another sign-in) happened while it worked, so it
/// saved nothing.
pub(super) const SESSION_CHANGED_WHILE_WAITING: &str =
    "Something changed on this computer while Beebeeb was working; nothing was saved. Try again.";

/// What a sign-in says when a Lock happened while it worked (Task 12 fix round 1, item 8).
pub(super) const SIGN_IN_LOCKED_MEANWHILE: &str = "Beebeeb was locked while signing in. Sign in again.";

/// What the recovery-phrase unlock says when a Lock happened while the phrase was being checked (a Lock keeps the person
/// signed in, so the phrase is all that is asked again).
pub(super) const RECOVERY_UNLOCK_LOCKED_MEANWHILE: &str =
    "Beebeeb was locked while the recovery phrase was being checked, so nothing was saved. Enter the phrase once more.";

/// What the Keychain unlock says when a Lock or a Sign-out happened while it unlocked (after a Sign-out there is nothing
/// left to unlock, so the advice is the neutral one; Task 12 fix round 2, item 5).
pub(super) const KEYCHAIN_UNLOCK_INTERRUPTED: &str = "Beebeeb was signed out or locked while unlocking. Try again.";

/// A transition's turn was refused: another session transition happened since it began. Each path says why in its own
/// words (one distinct sentence per path, so the frontend can tell them apart).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SessionChanged {
    pub(super) by_lock: bool,
}

impl SessionChanged {
    /// What moved `acct`'s generation last, as a refusal.
    pub(super) fn of(acct: &AccountRuntime) -> Self {
        Self {
            by_lock: acct.last_session_transition() == account::SessionTransition::Lock,
        }
    }

    pub(super) fn sign_in_sentence(self) -> &'static str {
        if self.by_lock {
            SIGN_IN_LOCKED_MEANWHILE
        } else {
            SESSION_CHANGED_WHILE_WAITING
        }
    }

    pub(super) fn recovery_unlock_sentence(self) -> &'static str {
        if self.by_lock {
            RECOVERY_UNLOCK_LOCKED_MEANWHILE
        } else {
            UNLOCK_ACCOUNT_CHANGED
        }
    }

    pub(super) fn keychain_unlock_sentence(self) -> &'static str {
        KEYCHAIN_UNLOCK_INTERRUPTED
    }
}

/// A sign-in's refusal, by default.
impl From<SessionChanged> for String {
    fn from(changed: SessionChanged) -> Self {
        changed.sign_in_sentence().to_string()
    }
}

/// Take the turn of the session transition that captured `turn` (Task 12, lead ruling 1): the session-write lock and,
/// on macOS and Linux, a refusal when any other transition of `acct` happened since (a Lock or a Sign-out, or another
/// sign-in that wrote first). On success the generation moves on and `turn` follows it, so the same transition's next
/// write passes and every OLDER transition is refused from now on. Windows orders its transitions with
/// `SESSION_TRANSITION` and its attempt fences and never refuses here; its generation still moves.
pub(super) fn claim_session_write(
    acct: &AccountRuntime,
    turn: &mut account::SessionGeneration,
) -> Result<SessionWrite, SessionChanged> {
    let write = check_session_turn(acct, turn)?;
    *turn = acct.advance_session_generation_holding_session_write_lock(account::SessionTransition::Write);
    Ok(write)
}

/// The same check as [`claim_session_write`] without moving the generation on: for a write that changes nothing a
/// sign-in decides on (a cached profile), or that moves it only when it turns out to change something (the naming of a
/// session, Task 12 fix round 1 item 5). The caller moves it on, holding the returned turn, when it does.
pub(super) fn check_session_turn(
    acct: &AccountRuntime,
    turn: &account::SessionGeneration,
) -> Result<SessionWrite, SessionChanged> {
    let writes = lock_session_keychain_writes();
    #[cfg(not(target_os = "windows"))]
    if !acct.session_unchanged_since(*turn) {
        tracing::info!("a session transition wrote nothing: another one happened since it began");
        return Err(SessionChanged::of(acct));
    }
    #[cfg(target_os = "windows")]
    let _ = turn;
    Ok(SessionWrite { _writes: writes })
}

/// Lock and Sign-out (`by`): every session transition that began before this call is refused from now on (it can no
/// longer pass [`claim_session_write`]). Always succeeds. The caller clears under the returned turn, or drops it at once.
pub(super) fn end_sessions_in_flight(acct: &AccountRuntime, by: account::SessionTransition) -> SessionWrite {
    let writes = lock_session_keychain_writes();
    acct.advance_session_generation_holding_session_write_lock(by);
    SessionWrite { _writes: writes }
}

/// Sign-out's last turn on macOS and Linux (Task 12 fix round 1, item 2): under the session-write lock, clear the
/// Keychain (both layouts), check that nothing of the account is left, clear memory once more (an unlock that began after
/// the sign-out's earlier memory clear may have put a session there), and only THEN move the generation on, before the
/// lock is released. So a transition that began before this point is refused at its next check, and one that begins
/// after it finds nothing to unlock. The caller still holds the engine slot (`_engine_slot`), so no start can read keys
/// in between. `best_effort`: the already-signed-out path, where a store that cannot answer is not a failure. The
/// generation moves whatever the result. `dropped` gets every session token this turn took (task 1886): what each
/// Keychain layout held before its clear, and a session put in memory while the sign-out ran, also when the turn fails.
#[cfg(not(target_os = "windows"))]
pub(super) fn finish_sign_out_turn(
    acct: &AccountRuntime,
    _engine_slot: &tokio::sync::MutexGuard<'_, Option<EngineRunner>>,
    best_effort: bool,
    dropped: &mut DroppedTokens,
) -> Result<(), String> {
    let (result, cleared_keys) = {
        let write = SessionWrite {
            _writes: lock_session_keychain_writes(),
        };
        let result = match clear_keychain_session_holding(&write, acct.id.as_str(), dropped) {
            Err(error) if best_effort => {
                tracing::warn!(%error, "already-signed-out sign-out: keychain session clear failed (nothing should be left); continuing");
                Ok(())
            }
            cleared => cleared,
        };
        let result = result.and_then(|()| ensure_keychain_holds_no_account(acct.id.as_str()));
        let taken = match acct.session.lock() {
            Ok(mut guard) => guard.take(),
            Err(poisoned) => {
                let taken = poisoned.into_inner().take();
                acct.session.clear_poison();
                taken
            }
        };
        let cleared_keys = taken.is_some();
        dropped.add(taken.map(|mut session| zeroize::Zeroizing::new(std::mem::take(&mut session.token))));
        acct.advance_session_generation_holding_session_write_lock(account::SessionTransition::SignOut);
        drop(write);
        (result, cleared_keys)
    };
    if cleared_keys {
        tracing::warn!("sign-out cleared a session that was put in memory while it ran");
        bump_vault_epoch();
    }
    result
}

/// Store a new session's token and vault key, with the email that goes with them (see
/// [`forget_account_email_in_keychain`]: the email kept for the session before is removed first). Only inside a checked
/// turn (`_write`).
pub(super) fn persist_session_to_keychain(
    _write: &SessionWrite,
    account_id: &str,
    token: &str,
    master_key: &[u8; 32],
    email: Option<&str>,
) -> Result<(), String> {
    let vault = AuthVault::new(platform_keychain_store_for(account_id));
    let token = SessionToken::new(token.to_string()).map_err(|e| keychain_error("session token", e))?;
    forget_account_email_in_keychain(account_id)?;
    vault
        .install_session(token)
        .map_err(|e| keychain_error("store session in Keychain", e))?;
    vault
        .store_wrapped_master_key(SecretBytes::new_master_key(master_key))
        .map_err(|e| keychain_error("store vault key in Keychain", e))?;
    // The session's own email, after its secrets: the Account page shows it after an auto-unlock on relaunch, and
    // the restore knows the stored session by it. None/empty (a sign-in that could not fetch the account record)
    // writes nothing, so the stored session has no email and stays unidentified until a fetch names it.
    persist_account_email_to_keychain(&vault, email)
}

/// Store a new session's token (its vault key comes later, from the recovery phrase), with the email that goes with
/// it (see [`forget_account_email_in_keychain`]). Only inside a checked turn (`_write`).
pub(super) fn persist_session_token_to_keychain(
    _write: &SessionWrite,
    account_id: &str,
    token: &str,
    email: Option<&str>,
) -> Result<(), String> {
    let vault = AuthVault::new(platform_keychain_store_for(account_id));
    let token = SessionToken::new(token.to_string()).map_err(|e| keychain_error("session token", e))?;
    forget_account_email_in_keychain(account_id)?;
    vault
        .install_session(token)
        .map_err(|e| keychain_error("store session in Keychain", e))?;
    persist_account_email_to_keychain(&vault, email)
}

/// Remove the account email the Keychain kept for the session stored before. Every write of a new session token
/// calls this BEFORE it stores the token, and writes the new session's own email (if it has one) after: the restore
/// knows a stored session by that email, so the Keychain must never hold one account's token or key next to another
/// account's email, not even when a later step of the write fails. A removal that fails fails the write, as a token
/// that cannot be stored does; an email that is not there is no failure.
pub(super) fn forget_account_email_in_keychain(account_id: &str) -> Result<(), String> {
    use keychain::AuthSecretStore as _;
    platform_keychain_store_for(account_id)
        .delete_account_email()
        .map_err(|e| keychain_error("remove the previous account email from Keychain", e))
}

/// Store the account email in the credential vault if one is provided.
///
/// Always returns `Ok(())`: a store that can't hold the email must never fail an otherwise-successful login, so a
/// persist error is logged and swallowed rather than surfaced. That is safe because the writers of a new session
/// remove the previous email first ([`forget_account_email_in_keychain`]): a failed write leaves NO email, and the
/// restore then treats the session as unidentified (no engine until a fetch names it) rather than as another
/// account. The email shows on the Account page after an auto-unlock and is what the restore knows the session by.
/// `None`/empty writes nothing.
pub(super) fn persist_account_email_to_keychain(
    vault: &AuthVault<keychain::PlatformKeychainStore>,
    email: Option<&str>,
) -> Result<(), String> {
    let Some(email) = email else {
        return Ok(());
    };
    if let Err(e) = vault.store_account_email(email) {
        tracing::warn!(error = %e, "could not persist account email to credential store (non-fatal)");
    }
    Ok(())
}

/// Store the vault key the recovery phrase proved. Only inside a checked turn (`_write`). The key is borrowed: no copy of
/// it is made here but the one the store wipes (`SecretBytes`). The store removes the previous key's owner tag first
/// (task 2447); the caller binds the new owner (`bind_vault_key_to_session_owner_in_keychain`).
pub(super) fn persist_vault_key_to_keychain(
    _write: &SessionWrite,
    account_id: &str,
    master_key: &[u8; 32],
) -> Result<(), String> {
    AuthVault::new(platform_keychain_store_for(account_id))
        .store_wrapped_master_key(SecretBytes::new_master_key(master_key))
        .map_err(|e| keychain_error("store vault key in Keychain", e))
}

pub(super) fn load_session_token_from_keychain(account_id: &str) -> Result<Option<String>, String> {
    let id_keyed = AuthVault::new(platform_keychain_store_for(account_id))
        .session_token()
        .map_err(|e| keychain_error("load session from Keychain", e))?;
    if let Some(token) = id_keyed {
        return Ok(Some(token.expose_for_request().to_string()));
    }
    // LEGACY READ FALLBACK (anti-lockout): the id-keyed lookup found nothing, so
    // fall back to the legacy flat store before concluding "no session." This
    // makes auto-unlock robust even if the migration no-op'd or failed.
    let legacy = AuthVault::new(keychain::legacy_platform_keychain_store())
        .session_token()
        .map_err(|e| keychain_error("load legacy session from Keychain", e))?;
    Ok(legacy.map(|t| t.expose_for_request().to_string()))
}

pub(super) fn load_session_from_keychain(account_id: &str, email: Option<String>) -> Result<Option<Session>, String> {
    // Try the id-segmented store first; if it has no session token, fall back to
    // the legacy flat store (anti-lockout READ FALLBACK §4D). The fallback is
    // read-only — it never writes — so it can never re-create orphan-able flat
    // entries; it only keeps a not-yet-migrated install auto-unlocking.
    let vault = AuthVault::new(platform_keychain_store_for(account_id));
    if vault
        .session_token()
        .map_err(|e| keychain_error("load session from Keychain", e))?
        .is_some()
    {
        return load_session_from_vault(vault, email);
    }
    // Id-keyed lookup empty → legacy fallback.
    let legacy = AuthVault::new(keychain::legacy_platform_keychain_store());
    if legacy
        .session_token()
        .map_err(|e| keychain_error("load legacy session from Keychain", e))?
        .is_some()
    {
        return load_session_from_vault(legacy, email);
    }
    Ok(None)
}

/// Shared tail of `load_session_from_keychain` for whichever store (id-keyed or
/// legacy) actually holds the session. Loads the token, unlocks the vault
/// (validating the 32-byte key), and recovers the email. Returns `Ok(None)` only
/// if the token vanished between the presence check and here (a race the caller
/// already filtered out); any unlock failure (incl. genuine `NotFound`) is an
/// `Err` so the startup restore can distinguish "no key on this PC" from success.
pub(super) fn load_session_from_vault<S: keychain::AuthSecretStore>(
    mut vault: AuthVault<S>,
    email: Option<String>,
) -> Result<Option<Session>, String> {
    let Some(token) = vault
        .session_token()
        .map_err(|e| keychain_error("load session from Keychain", e))?
    else {
        return Ok(None);
    };
    // Task 2447: whose key is this? Asked before the key is read, here, where every loader of token + key passes
    // (startup restore, unlock, re-sign-in; both layouts). Another account's key, or a tagged key next to a session
    // nobody named, is refused; an untagged key (from before the tags) loads as it always did.
    if !crate::auth_v2::key_owner::stored_key_ownership(&vault)
        .map_err(|e| keychain_error("read the vault key's owner", e))?
        .may_load()
    {
        return Err(crate::auth_v2::key_owner::KEY_NOT_THIS_ACCOUNTS.to_string());
    }
    vault
        .unlock()
        .map_err(|e| keychain_error("unlock vault key from Keychain", e))?;
    // Wiped when this function returns (Task 12 fix round 1, item 10): the one copy that outlives it is the session's.
    let mut master_key = zeroize::Zeroizing::new([0u8; 32]);
    master_key.copy_from_slice(vault.master_key().map_err(|e| keychain_error("read vault key", e))?);
    // Prefer an email already known in memory (e.g. carried through from a live
    // unlock). Otherwise recover the one persisted in the credential vault so the
    // Account page can show it after an auto-unlock on relaunch. A session stored
    // before email persistence existed simply yields `None` here (no error) —
    // backward-compatible by construction.
    let email = match email {
        Some(email) => Some(email),
        None => vault
            .account_email()
            .map_err(|e| keychain_error("read account email", e))?,
    };
    Ok(Some(Session {
        token: token.expose_for_request().to_string(),
        master_key: *master_key,
        email,
    }))
}

pub(super) fn keychain_session_present(account_id: &str) -> bool {
    let id_keyed = AuthVault::new(platform_keychain_store_for(account_id))
        .session_token()
        .map(|token| token.is_some())
        .unwrap_or(false);
    if id_keyed {
        return true;
    }
    // LEGACY READ FALLBACK (anti-lockout): report present if a legacy flat
    // session still exists (e.g. migration hasn't run yet), so onboarding routing
    // sees the signed-in state rather than falling back to the sign-in prompt.
    AuthVault::new(keychain::legacy_platform_keychain_store())
        .session_token()
        .map(|token| token.is_some())
        .unwrap_or(false)
}

// Windows' startup discard; on macOS and Linux the sign-out clears inside its last turn
// (`clear_keychain_session_holding`), and only tests call this.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(super) fn clear_keychain_session(account_id: &str) -> Result<(), String> {
    clear_keychain_session_into(account_id, &mut DroppedTokens::default())
}

/// [`clear_keychain_session`] that hands every session token it clears to `dropped` (Windows' sign-out, task 1886).
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(super) fn clear_keychain_session_into(account_id: &str, dropped: &mut DroppedTokens) -> Result<(), String> {
    let writes = SessionWrite {
        _writes: lock_session_keychain_writes(),
    };
    clear_keychain_session_holding(&writes, account_id, dropped)
}

/// [`clear_keychain_session`] inside a turn the caller already holds (the sign-out's last turn). Task 1886: `dropped`
/// gets the token each layout holds, read in this same turn just before that layout's clear, so a sign-out ends on the
/// server what it removes here (also when a later step fails: the caller ends only what is gone).
pub(super) fn clear_keychain_session_holding(
    _write: &SessionWrite,
    account_id: &str,
    dropped: &mut DroppedTokens,
) -> Result<(), String> {
    // Clear the id-segmented trio (the live layout post-migration).
    let mut vault = AuthVault::new(platform_keychain_store_for(account_id));
    dropped.add(stored_session_token(&vault));
    vault
        .clear_session()
        .map_err(|e| keychain_error("clear Keychain session", e))?;
    // Belt-and-braces: also clear any lingering legacy flat trio so a logout
    // leaves no recoverable secrets behind even if migration never ran (or the
    // legacy fallback had been serving the session). `clear_session` is
    // idempotent (NotFound → Ok), so this is safe when nothing legacy exists.
    let mut legacy = AuthVault::new(keychain::legacy_platform_keychain_store());
    dropped.add(stored_session_token(&legacy));
    legacy
        .clear_session()
        .map_err(|e| keychain_error("clear legacy Keychain session", e))
}

/// R8: a startup 401 drops only the rejected token, in the id-keyed and the legacy store. The vault key and the email
/// stay, so the same account signs in again without its recovery phrase. Only inside a checked turn (`_write`), so a
/// newer session's token is never the one removed.
#[cfg(not(target_os = "windows"))]
pub(super) fn clear_keychain_session_token(_write: &SessionWrite, account_id: &str) -> Result<(), String> {
    AuthVault::new(platform_keychain_store_for(account_id))
        .clear_session_token()
        .map_err(|e| keychain_error("clear Keychain session token", e))?;
    AuthVault::new(keychain::legacy_platform_keychain_store())
        .clear_session_token()
        .map_err(|e| keychain_error("clear legacy Keychain session token", e))
}

/// A startup 401 keeps the vault key and the email (R9), so a sign-out (and with it "Sign out and switch") must leave
/// none of the account's items for the next one: no token, no vault key, no email, in either layout. An item that cannot
/// be read counts as still there (fail closed); a store that cannot hold secrets has nothing to leave.
#[cfg(not(target_os = "windows"))]
pub(super) fn ensure_keychain_holds_no_account(account_id: &str) -> Result<(), String> {
    if keychain_session_trace(account_id) || keychain_vault_key_present(account_id) || keychain_email_trace(account_id)
    {
        return Err(SIGN_OUT_KEYCHAIN_NOT_CLEARED.to_string());
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
pub(super) const SIGN_OUT_KEYCHAIN_NOT_CLEARED: &str = "Sign-out paused: Beebeeb couldn’t remove the previous account’s keys from this computer. Try again; if it keeps happening, restart Beebeeb.";

/// Read the segmented keychain account-email credential WITHOUT unlocking
/// (email is metadata, not key material — no lock-state gate). `None` on
/// absence or on a store that cannot answer (the Linux fail-closed stub).
pub(super) fn keychain_account_email(account_id: &str) -> Option<String> {
    AuthVault::new(platform_keychain_store_for(account_id))
        .account_email()
        .ok()
        .flatten()
}

/// The email to remember for the sign-in prefill at sign-out time, in
/// precedence order: the in-memory session's email, then the mirrored
/// per-account `auth_email`, then the keychain account-email credential.
/// MUST be called before `clear_keychain_session` erases the keychain copy.
pub(super) fn signout_email_to_preserve(acct: &crate::account::AccountRuntime) -> Option<String> {
    acct.session
        .lock()
        .ok()
        .and_then(|guard| guard.as_ref().and_then(|s| s.email.clone()))
        .or_else(|| acct.auth_email.lock().ok().and_then(|guard| guard.clone()))
        .or_else(|| keychain_account_email(acct.id.as_str()))
}

/// Pure core of [`persist_last_signed_in_email`]: stage the prefill email
/// onto a config. `None` (nothing signed in) and unchanged values are
/// no-ops, so sign-out never rewrites `desktop.toml` just for the prefill.
/// Returns whether the config changed and needs saving.
pub(super) fn stage_last_signed_in_email(cfg: &mut DesktopConfig, email: Option<&str>) -> bool {
    match email {
        None => false,
        Some(email) if cfg.last_signed_in_email.as_deref() == Some(email) => false,
        Some(email) => {
            cfg.last_signed_in_email = Some(email.to_string());
            true
        }
    }
}

/// Best-effort: remember the last signed-in email so the onboarding sign-in
/// form can prefill it after a sign-out / startup-401 auto sign-out (the
/// keychain account-email credential is erased by `clear_keychain_session`,
/// so this desktop.toml field is the only surviving copy). A config failure
/// must never fail sign-out — log and continue.
pub(super) fn persist_last_signed_in_email(email: Option<String>) {
    let Some(email) = email else { return };
    // Task 1889: one load-change-save under the config-write lock (the sign-out runs beside the window's saves).
    let result = DesktopConfig::update(|cfg| (stage_last_signed_in_email(cfg, Some(&email)), ()));
    if let Err(error) = result {
        tracing::warn!(
            %error,
            "could not persist the last signed-in email for sign-in prefill (best-effort)"
        );
    }
}

/// What a sign-out that is an account switch says when it could not forget the previous account's email. Nothing was
/// torn down, so trying again is safe.
pub(super) const SIGN_OUT_EMAIL_NOT_FORGOTTEN: &str = "Sign-out paused: Beebeeb couldn’t remove the previous account’s email from this computer. Try again; if it keeps happening, restart Beebeeb.";

/// An account switch's sign-out (spec §5.6: "The switch leaves no vault key and no account email behind, or it
/// stops"): forget the sign-in prefill (`last_signed_in_email`, which a sign-out and a startup 401 keep) before anything
/// is torn down. Unlike [`persist_last_signed_in_email`] this is not best-effort: a `desktop.toml` that cannot be read
/// or written stops the switch. An email that is not there is written nothing.
pub(super) fn forget_last_signed_in_email() -> Result<(), String> {
    let path = DesktopConfig::path().map_err(|error| {
        tracing::warn!(%error, "account switch: desktop.toml could not be read to forget the previous email");
        SIGN_OUT_EMAIL_NOT_FORGOTTEN.to_string()
    })?;
    // Task 1889: one load-change-save under the config-write lock. The two log lines stay apart: a read that
    // failed and a write that failed are different things to look for.
    DesktopConfig::try_update_at(&path, |cfg| (cfg.last_signed_in_email.take().is_some(), ())).map_err(|error| {
        match error {
            config::UpdateError::Read(error) => {
                tracing::warn!(%error, "account switch: desktop.toml could not be read to forget the previous email")
            }
            config::UpdateError::Write(error) => {
                tracing::warn!(%error, "account switch: desktop.toml could not be written to forget the previous email")
            }
        }
        SIGN_OUT_EMAIL_NOT_FORGOTTEN.to_string()
    })
}

/// Result of the one bounded startup session probe (`GET /api/v1/auth/me`).
///
/// The classification is deliberately three-valued: only a DEFINITIVE HTTP
/// 401 proves the stored token is dead. Everything else — network error,
/// timeout, 5xx, any other status — is [`StartupSessionCheck::Inconclusive`]
/// and must fail OPEN (offline users must never be logged out by startup).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum StartupSessionCheck {
    /// `GET /auth/me` answered 2xx — the stored token is valid. `profile` is the account the server named in that
    /// answer (`None` when the body could not be read: still authorized, only the profile is missing).
    Authorized {
        profile: Option<account_dto::AccountProfile>,
    },
    /// `GET /auth/me` answered exactly 401 — the stored token is definitively
    /// rejected; the session may be discarded (auto sign-out).
    Unauthorized,
    /// Anything else (network error, timeout, 5xx, other status). Never
    /// authorizes discarding the session.
    Inconclusive(String),
}

/// One bounded `GET /api/v1/auth/me` with the stored session token (the same
/// endpoint+auth shape [`ApiClient::account_profile`] uses, via the ad-hoc
/// reqwest pattern the logout revocation uses). `base_url` is a parameter so
/// tests can point this at a loopback mock; production passes
/// [`runner::api_base_url`]. Total cost is capped at 5 seconds — this runs
/// during startup.
///
/// The bearer token is only sent over the wire, never logged.
pub(super) async fn probe_startup_session(base_url: &str, token: &str) -> StartupSessionCheck {
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .default_headers(api_client::provenance_headers())
        .build()
    {
        Ok(client) => client,
        Err(error) => return StartupSessionCheck::Inconclusive(format!("client build: {error}")),
    };
    let request = client.get(format!("{base_url}/api/v1/auth/me")).bearer_auth(token);
    let response = match tokio::time::timeout(std::time::Duration::from_secs(5), request.send()).await {
        Ok(Ok(response)) => response,
        Ok(Err(error)) => return StartupSessionCheck::Inconclusive(format!("network error: {error}")),
        Err(_) => return StartupSessionCheck::Inconclusive("probe timed out after 5s".to_string()),
    };
    match response.status() {
        reqwest::StatusCode::UNAUTHORIZED => StartupSessionCheck::Unauthorized,
        status if status.is_success() => {
            let profile = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                response.json::<account_dto::AccountProfile>(),
            )
            .await
            .ok()
            .and_then(Result::ok);
            StartupSessionCheck::Authorized { profile }
        }
        status => StartupSessionCheck::Inconclusive(format!("HTTP {status}")),
    }
}

/// Apply the auto sign-out for a startup session the server definitively
/// rejected with 401: clear the keychain session trio (exactly like a real
/// sign-out's credential step — wrapped-master-key semantics untouched,
/// `clear_keychain_session` is the same call) and drop the auth flags so the
/// app boots signed-out and the SignedOutGate routes to onboarding/sign-in.
/// The in-memory session is never installed and the engine never started on
/// this path (the caller checks before doing either).
///
/// `email` is the rejected session's email — persisted for the sign-in
/// prefill BEFORE the keychain clear erases the keychain copy (best-effort).
///
/// Best-effort on the keychain clear: a store that cannot answer (the Linux
/// fail-closed stub) must not panic startup — log and continue, the flags
/// still route the app to signed-out.
#[cfg(target_os = "windows")]
pub(super) fn discard_unusable_startup_session(
    state: &AppState,
    acct: &crate::account::AccountRuntime,
    email: Option<String>,
) {
    tracing::info!(
        account_id = acct.id.as_str(),
        "stored session token was rejected by the server (HTTP 401) at startup; \
         clearing the stored session so the app boots signed out"
    );
    // The session was never installed in memory, so the caller passes the
    // restored session's email directly; fall back to the per-account mirror
    // / keychain credential while they still exist (short-circuits when the
    // session email is known).
    persist_last_signed_in_email(email.or_else(|| signout_email_to_preserve(acct)));
    if let Err(error) = clear_keychain_session(acct.id.as_str()) {
        tracing::warn!(
            %error,
            "could not clear the server-rejected keychain session trio at startup (best-effort)"
        );
    }
    set_auth_present(state, false);
    set_auth_email(state, None);
}

/// R8 (spec 2026-10-06): the server rejected the stored token at startup. Drop only that token, keep the vault key and
/// the email, and boot signed out. The next sign-in as the same account swaps in a new token and needs no recovery
/// phrase (`reauth_in_place`). Beebeeb stays in Finder (this is not a sign-out by choice). Another account goes through
/// the switch: full sign-out first. Inside the restore's own turn (`turn`): when another session transition happened
/// while the probe waited (a sign-in may have stored a new token since), nothing is removed and no flag changes.
#[cfg(not(target_os = "windows"))]
pub(super) fn discard_unusable_startup_session(
    state: &AppState,
    acct: &crate::account::AccountRuntime,
    email: Option<String>,
    turn: &mut account::SessionGeneration,
) {
    let Ok(write) = claim_session_write(acct, turn) else {
        tracing::info!("the server rejected the stored token at startup, but the session changed since; left as it is");
        return;
    };
    tracing::info!(
        account_id = acct.id.as_str(),
        "stored session token was rejected by the server (HTTP 401) at startup; dropping the token, keeping the keys (R8)"
    );
    persist_last_signed_in_email(email.or_else(|| signout_email_to_preserve(acct)));
    if let Err(error) = clear_keychain_session_token(&write, acct.id.as_str()) {
        tracing::warn!(%error, "could not clear the server-rejected session token at startup (best-effort)");
    }
    set_auth_present(state, false);
    set_auth_email(state, None);
}

/// On launch, resume a fully signed-in **and unlocked** session when the
/// platform credential store still holds BOTH the session token and the
/// vault master key.
///
/// This is the persistence guarantee desktop users expect from OneDrive /
/// Dropbox: quit + relaunch returns to a working, unlocked state with no
/// recovery-phrase prompt. The stored `wrapped-master-key` is the raw
/// 32-byte vault root key, protected at rest by the per-user OS credential
/// vault (DPAPI on Windows, Keychain on macOS) — reading it back is all that
/// is required to unlock; there is no passphrase to enter.
///
/// `load_session_from_keychain` is the same path `unlock_vault` uses: it
/// loads the token, calls `AuthVault::unlock()` (which validates the key is
/// 32 bytes and flips the vault to Unlocked), and returns the in-memory
/// `Session`. We then perform exactly the side-effects `apply_session` /
/// `unlock_vault` perform — stash the session in memory, mark auth present,
/// and start the engine for the configured sync root — so the resumed state
/// is indistinguishable from a fresh in-session unlock.
///
/// This function deliberately does NOTHING (no error surfaced) when:
///   - there is no stored session token (never signed in / logged out) —
///     `load_session_from_keychain` returns `Ok(None)`; or
///   - the token IS present but the master key is genuinely absent —
///     `AuthVault::unlock()` returns `AuthStoreError::NotFound`, which
///     `load_session_from_keychain` surfaces as `Err(..)` (NOT `Ok(None)`),
///     logged at `debug!`.
/// The second case is the legitimate "this PC doesn't have your keys" state
/// where onboarding's recovery-phrase step must still appear. `auth_present`
/// is already seeded from `keychain_session_present()` in `setup()`, so a
/// token-only restore still reports `logged_in: true` and routes to unlock.
/// A genuine credential-store/backend failure (anything other than
/// `NotFound`) is logged at `warn!` instead — it is unexpected but must still
/// not block startup, since onboarding can recover.
pub(super) async fn restore_session_on_startup(app: &tauri::AppHandle) {
    restore_session_inner(app).await;
    // Whichever way the restore returned (refused on Windows, no account, a session already installed, nothing in
    // the Keychain, a revoked token, ...), the upgrade's one-time adoption window is over. The restore adopts on
    // what it loaded before it gets here; this closes the window on every path that did not.
    adopt_unbound_local_data_at_startup(None);
}

/// The startup restore proper (see [`restore_session_on_startup`], which shuts the adoption window after it).
pub(super) async fn restore_session_inner(app: &tauri::AppHandle) {
    // Spec 2026-10-10 §15 question 3: a sign-out quit between its keep step and its purge left Finder accepts closed on
    // disk. Reopen them before this restore can start an engine.
    #[cfg(target_os = "macos")]
    reopen_finder_accepts_at_startup();
    #[cfg(target_os = "windows")]
    let _transition = SESSION_TRANSITION.lock().await;
    #[cfg(target_os = "windows")]
    if !SESSION_COMMANDS.startup_restore_allowed() || windows_cf::ensure_reactivation_allowed().is_err() {
        return;
    }
    let state = app.state::<AppState>();
    // `synthesize_single_account` runs in `setup()` before this task is
    // spawned, so the active account always resolves; bail defensively if not.
    let Ok(acct) = state.active_account() else {
        tracing::warn!("no active account during startup restore");
        return;
    };
    if !restore_stored_session(&state, &acct, &runner::api_base_url()).await {
        return;
    }
    // Windows keeps today's rule: a restored session with no email is named by the server before the engine start.
    #[cfg(target_os = "windows")]
    if acct
        .session
        .lock()
        .map(|guard| guard.as_ref().is_some_and(|session| session.email.is_none()))
        .unwrap_or(false)
    {
        establish_session_identity(&state, &acct, &runner::api_base_url(), None).await;
    }
    if let Err(error) = start_engine_if_possible(
        app.clone(),
        &state,
        #[cfg(target_os = "windows")]
        &_transition,
    )
    .await
    {
        tracing::warn!(%error, "startup engine refused");
    }
    // macOS and Linux (fix round 1, item 7): a restored session the probe could not name asks the server once more in
    // the background, through the account's single flight, and its engine start follows a successful naming. The
    // start above never waits for it.
    #[cfg(not(target_os = "windows"))]
    tauri::async_runtime::spawn(identify_then_start(app.clone(), IdentifyPace::Now));
}
