//! Multi-account runtime model — Phase 0 ("list of one").
//!
//! This module introduces the per-account indirection that future phases need
//! WITHOUT changing any observable behaviour at N=1. Today the app runs exactly
//! one account; [`synthesize_single_account`] mints that single account at
//! startup and the rest of the app reaches it through
//! [`crate::AppState::active_account`].
//!
//! Phase boundaries (do not cross them in Phase 0):
//!   - Phase 0 (this): in-memory indirection only. One synthesized account.
//!     No on-disk shape change, no new keychain entries, no second engine.
//!   - Phase 1: persist the account id + per-account keychain segments + an
//!     `accounts[]` projection on disk. **See the LOUD note on
//!     [`synthesize_single_account`] — the ephemeral id MUST be persisted then.**
//!   - Phase 2: N live engines + per-account `windows_cf` registration.
//!
//! Why the per-account state (`session`, `engine`, `engine_state`,
//! `sync_paused`, `cached_profile`, `auth_email`) lives here and not on
//! `AppState`: it is the state that will become 1-per-account in Phase 2. The
//! state that stays on `AppState` is process-global (`auth_present`) or
//! login-in-flight with no account yet (`pending_2fa`).

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::path::PathBuf;

use crate::AppState;
use crate::Session;
use crate::account_dto::AccountProfile;
use crate::config::DesktopConfig;
use crate::runner::EngineRunner;

/// Stable identity for one account in the runtime registry.
///
/// **UUID v4** (not FNV-of-path, not email-derived): the sync root is mutable
/// (`pick_sync_root`) and the email is both mutable and PII, so neither makes a
/// safe stable key. A random v4 has no such coupling.
///
/// `Hash + Eq + Clone + Serde` so it can key maps and round-trip on disk in
/// Phase 1. In Phase 0 it is **process-ephemeral** (see
/// [`synthesize_single_account`]).
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct AccountId(pub String);

impl AccountId {
    /// Mint a fresh random account id.
    pub fn new_v4() -> Self {
        AccountId(uuid::Uuid::new_v4().to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Per-account **runtime projection** of the loaded [`DesktopConfig`].
///
/// This is NOT a new serde shape on disk — `desktop.toml` keeps its current
/// single-document layout (decision 0800 risk #2). `AccountConfig` is built
/// from the one loaded `DesktopConfig` and carries only the per-account fields
/// (decision 0800's device-global vs per-account split). In Phase 0 it is
/// minimal/barely-consumed — Group D handlers still read `DesktopConfig`
/// directly; this type exists to establish the model that Phase 1 routes
/// through.
#[derive(Debug, Clone)]
pub struct AccountConfig {
    pub id: AccountId,
    pub sync_root: Option<PathBuf>,
    pub excluded_folder_ids: Option<Vec<String>>,
    pub known_folder_backup: Vec<String>,
    pub files_on_demand: Option<bool>,
    pub sync_mode: Option<String>,
    pub finder_install_status: Option<String>,
    pub finder_install_last_error: Option<String>,
    pub finder_install_last_attempt_at: Option<i64>,
    pub finder_install_reason_category: Option<String>,
}

impl AccountConfig {
    /// Project the per-account fields out of the single loaded `DesktopConfig`.
    ///
    /// Clones the relevant fields — `DesktopConfig` is the source of truth on
    /// disk; this is a read-only snapshot for the runtime account.
    pub fn from_desktop_config(id: AccountId, cfg: &DesktopConfig) -> Self {
        Self {
            id,
            sync_root: cfg.sync_root.clone(),
            excluded_folder_ids: cfg.excluded_folder_ids.clone(),
            known_folder_backup: cfg.known_folder_backup.clone(),
            files_on_demand: cfg.files_on_demand,
            sync_mode: cfg.sync_mode.clone(),
            finder_install_status: cfg.finder_install_status.clone(),
            finder_install_last_error: cfg.finder_install_last_error.clone(),
            finder_install_last_attempt_at: cfg.finder_install_last_attempt_at,
            finder_install_reason_category: cfg.finder_install_reason_category.clone(),
        }
    }
}

/// The live, per-account state moved off `AppState`.
///
/// Holds exactly the state that becomes 1-per-account in Phase 2:
///   - `session` — the authoritative `Option<Session>`. **`Session` is `!Clone`
///     by design (one master key per process).** Never clone the `Session` out;
///     callers lock this mutex and read it under the borrow (or copy the
///     `[u8; 32]` master key into an `ApiClient`, the existing pattern). This
///     struct therefore deliberately does **not** derive `Clone`.
///   - `engine` / `engine_state` / `sync_paused` — the runner handle + its
///     last-seen status string + the runtime pause flag.
///   - `cached_profile` — the `/auth/me` payload cached at login.
///   - `auth_email` — the signed-in address mirrored for the Account page.
///
/// Accessed via `Arc<AccountRuntime>` from `AppState::active_account()`; callers
/// lock the inner mutexes. The `Arc` shares the runtime; it never duplicates the
/// session.
pub struct AccountRuntime {
    pub id: AccountId,
    pub session: Mutex<Option<Session>>,
    pub engine: tokio::sync::Mutex<Option<EngineRunner>>,
    pub engine_state: Mutex<String>,
    pub sync_paused: Arc<AtomicBool>,
    pub cached_profile: Mutex<Option<AccountProfile>>,
    pub auth_email: Mutex<Option<String>>,
}

impl AccountRuntime {
    /// A fresh account with no session and a stopped engine — the exact
    /// per-account mirror of the old `AppState::default` live-state fields
    /// (`session: None`, `engine: None`, `engine_state: "stopped"`,
    /// `sync_paused: false`, `cached_profile: None`, `auth_email: None`).
    pub fn new(id: AccountId) -> Self {
        Self {
            id,
            session: Mutex::new(None),
            engine: tokio::sync::Mutex::new(None),
            // Mirrors the old `AppState::default` — once the runner spawns and
            // emits its first event, the listener overwrites this.
            engine_state: Mutex::new("stopped".to_string()),
            sync_paused: Arc::new(AtomicBool::new(false)),
            cached_profile: Mutex::new(None),
            auth_email: Mutex::new(None),
        }
    }
}

/// Migration shim: ensure the registry holds exactly one synthesized account.
///
/// Idempotent — a no-op if `accounts` is already non-empty, so calling it twice
/// (or after a fresh login already created the account) can never spawn a
/// second runtime. Pushes one [`AccountRuntime`] mirroring the old
/// `AppState::default` live state and sets it active.
///
/// Call this in `setup()` AFTER `.manage(AppState::default())` takes effect and
/// BEFORE `restore_session_on_startup` (so the restore lands its keychain
/// session in `accounts[0].session`) and BEFORE first-launch detection. At N=1
/// this is invisible: fresh install → empty `accounts[0]`, onboarding opens as
/// today; existing install → the restore loads the keychain session INTO
/// `accounts[0]` and the single `DesktopConfig` still yields the same sync root.
///
/// ───────────────────────────────────────────────────────────────────────────
/// ✅  PHASE 1: the id is now PERSISTED, not minted here.  ✅
/// The caller (`setup()`) computes the account id FIRST via
/// `DesktopConfig::ensure_account_id` — which mints a fresh UUID v4 and writes it
/// to `desktop.toml` on the first login, then returns the same value on every
/// subsequent launch — and passes it in. This closes the Phase-0
/// orphan-on-relaunch hole: because the keychain secrets are now segmented under
/// this id (`io.beebeeb.app/<id>/<leaf>`), a fresh-each-launch id would strand
/// every previous run's secrets under a dead id. Persisting the id BEFORE any
/// segment is written under it is the invariant that keeps auto-unlock working
/// across relaunches.
///
/// Still idempotent + invisible at N=1: a no-op if `accounts` is already
/// non-empty, so a second call (or a fresh login that already created the
/// account) can never spawn a second runtime.
/// ───────────────────────────────────────────────────────────────────────────
pub fn synthesize_single_account(state: &AppState, id: AccountId) {
    let mut accounts = match state.accounts.lock() {
        Ok(guard) => guard,
        Err(_) => return,
    };
    if !accounts.is_empty() {
        return;
    }
    // Persisted id, supplied by the caller (see the Phase-1 note above).
    accounts.push(Arc::new(AccountRuntime::new(id.clone())));
    if let Ok(mut active) = state.active_account_id.lock() {
        *active = Some(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AppState;

    /// A fixed id stands in for the value `DesktopConfig::ensure_account_id`
    /// persists, so these tests don't depend on a random mint.
    fn fixed_id() -> AccountId {
        AccountId("acct-fixed-0001".to_string())
    }

    #[test]
    fn synthesize_is_idempotent() {
        let state = AppState::default();
        synthesize_single_account(&state, fixed_id());
        synthesize_single_account(&state, fixed_id());
        assert_eq!(
            state.accounts.lock().unwrap().len(),
            1,
            "a second synthesize call must be a no-op"
        );
    }

    #[test]
    fn synthesize_uses_the_supplied_persisted_id() {
        // The synthesized account must carry exactly the id the caller passes
        // (the persisted desktop.toml account_id), not a freshly-minted one —
        // this is what keeps the keychain segments stable across relaunches.
        let state = AppState::default();
        synthesize_single_account(&state, fixed_id());
        let acct = state.active_account().expect("resolves after synthesis");
        assert_eq!(acct.id, fixed_id(), "runtime id must equal the supplied id");
        assert_eq!(
            state.active_account_id.lock().unwrap().as_ref(),
            Some(&fixed_id()),
            "active id must equal the supplied id"
        );
    }

    #[test]
    fn active_account_after_synthesis_is_default_shape() {
        let state = AppState::default();
        synthesize_single_account(&state, fixed_id());
        let acct = state
            .active_account()
            .expect("active account resolves after synthesis");
        assert_eq!(
            *acct.engine_state.lock().unwrap(),
            "stopped",
            "fresh runtime engine_state must start 'stopped'"
        );
        assert!(
            acct.session.lock().unwrap().is_none(),
            "fresh runtime must have no session"
        );
        assert!(
            !acct
                .sync_paused
                .load(std::sync::atomic::Ordering::Relaxed),
            "fresh runtime must not be paused"
        );
    }

    #[test]
    fn active_account_empty_registry_errs() {
        // Pre-synthesis (fresh `AppState`) has no accounts → Err.
        let state = AppState::default();
        assert!(
            state.active_account().is_err(),
            "active_account must error before synthesize_single_account runs"
        );
    }
}
