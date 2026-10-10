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

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};

use crate::AppState;
use crate::Session;
use crate::account_dto::AccountProfile;
use crate::config::DesktopConfig;
use crate::runner::{AuthHealth, EngineRunner};

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

/// A point in an account's sequence of session transitions: every sign-in, re-sign-in, unlock, lock and sign-out, the
/// startup restore and its discard of a token the server rejected, and the naming of a session the server identified.
/// Work that belongs to the session of the moment captures it first ([`AccountRuntime::session_generation`]) and,
/// before it acts on its result, asks whether that session is still the one it started with
/// ([`AccountRuntime::session_unchanged_since`]). A result whose session changed is dropped.
///
/// The session writers in `lib.rs` (the Keychain and the session in memory) go further: a write is allowed only under
/// the session-write lock and only while the generation its transition captured is still current, and it moves the
/// generation on, so an older transition that is still in flight can never write after it (and none at all after a
/// Lock or a Sign-out has returned).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionGeneration(u64);

/// What moved an account's [`SessionGeneration`] last, so a transition that is refused can say why in words that fit
/// (Task 12 fix round 1, item 8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionTransition {
    /// A session was written: a sign-in, an unlock, the restore, the naming of a session.
    Write = 0,
    Lock = 1,
    SignOut = 2,
}

/// One flight of the request that asks the server who an unidentified session is (Task 12 fix round 1, item 4): at most
/// one at a time per account, and app activation asks at most once a minute.
#[derive(Debug, Default)]
pub(crate) struct IdentifyFlight {
    pub(crate) in_flight: bool,
    pub(crate) last_started: Option<std::time::Instant>,
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
///   - `auth_health` — the consecutive-401 streak fed by the engine's
///     heartbeat + sync-tick API calls, read by `sync_status` as
///     `auth_expired` (task 1546 finding 5).
///   - `engine_stop_unconfirmed` — set when a sign-out could not CONFIRM
///     the engine task terminated (Bug A2): the consumed handle leaves an
///     empty `engine` slot, so without this flag a sign-out RETRY would
///     sail past the stop gate (slot looks idle) into the cross-account
///     purge while the old engine may still be running. In-memory only —
///     a process restart is the documented remedy (and re-resets it).
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
    pub auth_health: Arc<AuthHealth>,
    pub engine_stop_unconfirmed: AtomicBool,
    /// See [`SessionGeneration`]. Moved on only by `lib.rs`, under the session-write lock.
    session_generation: AtomicU64,
    /// What moved it last (a [`SessionTransition`] as a `u8`).
    last_transition: AtomicU8,
    /// The identify request's single flight and its debounce (see [`IdentifyFlight`]).
    pub(crate) identify_flight: Mutex<IdentifyFlight>,
    /// The naming could not write the session's email to the Keychain; the next naming trigger writes it again
    /// (Task 12 fix round 2, item 3).
    pub(crate) keychain_email_pending: AtomicBool,
    /// The last binding refusal of an engine start for the current session (Task 12 fix round 2, ruling P), shown as
    /// `sync_status.engine_refusal`. A start that runs clears it, and so does every session transition.
    pub(crate) engine_refusal: Mutex<Option<crate::account_binding::Refusal>>,
    /// Lead ruling F9 (spec 2026-10-06 R8): how many sign-ins have asked for the operations paused for `auth` to be due
    /// again. A sign-in that puts a session in memory moves it on in its write turn, after the session is there.
    pub(crate) auth_resume_asked: AtomicU64,
    /// The ask the last engine start served (it made those operations due before the engine existed). Written only
    /// under the engine slot.
    pub(crate) auth_resume_done: AtomicU64,
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
            auth_health: Arc::new(AuthHealth::new()),
            engine_stop_unconfirmed: AtomicBool::new(false),
            session_generation: AtomicU64::new(0),
            last_transition: AtomicU8::new(0),
            identify_flight: Mutex::new(IdentifyFlight::default()),
            keychain_email_pending: AtomicBool::new(false),
            engine_refusal: Mutex::new(None),
            auth_resume_asked: AtomicU64::new(0),
            auth_resume_done: AtomicU64::new(0),
        }
    }

    /// The account's current [`SessionGeneration`]. Capture it before starting work for the current session.
    pub fn session_generation(&self) -> SessionGeneration {
        SessionGeneration(self.session_generation.load(Ordering::SeqCst))
    }

    /// Whether no session transition has happened since `generation` was captured: the session is still the one the
    /// work started with, and its result may be used.
    pub fn session_unchanged_since(&self, generation: SessionGeneration) -> bool {
        self.session_generation() == generation
    }

    /// Move the generation on: a session transition (`by`) happened. The long name is the contract: call it only while
    /// holding `lib.rs`'s session-write lock, so a check and the write that follows it can never be split by another
    /// transition.
    pub(crate) fn advance_session_generation_holding_session_write_lock(
        &self,
        by: SessionTransition,
    ) -> SessionGeneration {
        self.last_transition.store(by as u8, Ordering::SeqCst);
        let moved = SessionGeneration(self.session_generation.fetch_add(1, Ordering::SeqCst).wrapping_add(1));
        // An engine-start refusal belongs to the session it was found for: any transition ends it (ruling P). Cleared
        // after the move, so a start that records one checks the generation under this same mutex and never records
        // over a transition.
        *self
            .engine_refusal
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        // Fix round 3, M5: a Keychain email write still owed belongs to the session a Lock or a Sign-out ends. A Write
        // keeps it: the naming that marks a failed write moves the generation with a Write right after.
        if matches!(by, SessionTransition::Lock | SessionTransition::SignOut) {
            self.keychain_email_pending.store(false, Ordering::SeqCst);
        }
        moved
    }

    /// What moved the generation last.
    pub(crate) fn last_session_transition(&self) -> SessionTransition {
        match self.last_transition.load(Ordering::SeqCst) {
            1 => SessionTransition::Lock,
            2 => SessionTransition::SignOut,
            _ => SessionTransition::Write,
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
        let acct = state.active_account().expect("active account resolves after synthesis");
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
            !acct.sync_paused.load(std::sync::atomic::Ordering::Relaxed),
            "fresh runtime must not be paused"
        );
    }

    /// The session generation (Task 12, lead ruling 11): work captures it, and asks before it uses its result whether a
    /// session transition happened since. Every transition moves it on, never back, and each account has its own.
    #[test]
    fn the_session_generation_tells_whether_the_session_changed_since_it_was_captured() {
        let acct = AccountRuntime::new(fixed_id());
        let other = AccountRuntime::new(AccountId("acct-other".into()));
        let others = other.session_generation();
        let captured = acct.session_generation();
        assert!(acct.session_unchanged_since(captured), "nothing happened yet");
        let after = acct.advance_session_generation_holding_session_write_lock(SessionTransition::Write);
        assert!(
            !acct.session_unchanged_since(captured),
            "a transition happened since it was captured"
        );
        assert!(
            acct.session_unchanged_since(after),
            "the generation the transition moved to is current"
        );
        assert_eq!(acct.session_generation(), after);
        assert_ne!(
            acct.advance_session_generation_holding_session_write_lock(SessionTransition::Write),
            after,
            "it never comes back to an earlier value"
        );
        assert!(!acct.session_unchanged_since(after));
        assert!(
            other.session_unchanged_since(others),
            "another account's work is not touched by these transitions"
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
