//! One engine handle for a host without a UI toolkit (the native shell, a test): restore the stored session, say
//! which screen the window shows, run the sign-in steps, start and stop sync, list a folder.
//!
//! Threading (also in `README.md`): every method BLOCKS the calling thread until it is done, on the handle's own
//! tokio runtime. Call them from a worker thread, never from the UI thread. The sync engine itself runs on that
//! runtime's threads and reports through the [`EngineHost`] given to [`Engine::open`] (a
//! [`crate::host::ChannelHost`] queues each event and wakes the UI loop).
//!
//! The rules are the ones the Tauri shell uses, from `crate::signin`: the gate (`gate`), whose key is stored next
//! to whose session (`key_owner`, task 2447), the loader with its owner check (`store`), the recovery proof
//! (`recovery`) and the keys of an engine start (`start`). The ORDER of the writes here mirrors the shell's
//! commands (`store_first_sign_in`, `store_recovered_vault_key`, `install_recovered_session`); it is this
//! module's own code, not theirs.
//!
//! Must not: log, print or return a token, a key, a password, a code or a phrase; hand keys to an engine start
//! from anywhere but the session in memory; start an engine before the local data is bound to the session's
//! account; reset or delete local data (a start onto another account's data is refused, on every OS); hand a shell
//! the local state by any way but the account-bound handle (`owned_state`, task 2488).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use zeroize::Zeroizing;

use plan_gate::PlanKnown;
use crate::account::{AccountId, AccountRuntime, Accounts, SessionGeneration, SessionTransition};
use crate::account_binding::{self, Identity};
use crate::account_dto::AccountProfile;
use crate::auth_transport;
use crate::drive_v2::dto::DriveItem;
use crate::drive_v2::model::{self, Sort};
use crate::host::EngineHost;
use crate::identity::EngineIdentity;
use crate::keychain::{AuthVault, SecretBytes, SessionToken};
use crate::runner::{EngineRunner, ResumeTicket};
use crate::session::Session;
use crate::signin::gate::{self, GateState, NextAfterSignIn, SignInProblem};
use crate::signin::key_owner::{self, StoredKey};
use crate::signin::password::{self, PasswordLegs};
use crate::signin::{recovery, start, store};
use crate::state_db::StateDb;
use crate::state_paths;
use crate::sync_root_owner::Registration;
use crate::sync_root_port::{OsSyncRoots, SyncRootPort};
use crate::sync_view::{Explorer, Start, SyncView, ViewCell, ViewHost};
use beebeeb_core::auth_flow::sign_in::PendingSecondFactor;
use lock_api::SessionMoved;

mod plan_gate;
pub mod account_actions_api;
pub mod account_api;
pub mod account_email_api;
pub mod account_email_resend_api;
pub mod account_password_api;
pub mod account_twofactor_api;
pub mod backup_devices_api;
pub(crate) mod credential_stores;
mod owed_online;
mod session_revoke;
mod session_watch;
mod unfinished_sign_out;
mod account_switch;
use session_revoke::revoke;
pub mod account_twofactor_status_api;
pub mod backups_api;
pub mod bindings;
// Files in two versions and the answer to one, for the native Files window.
pub mod conflicts_api;
// Explorer's "Free up space" command asks this handle, and a stop removes its binding (task 2490).
pub(crate) mod explorer_api;
// The sidebar's figures for the native Files window: files kept on this PC, the storage split.
pub mod files_facts_api;
pub mod kept_api;
mod kept_key_proof;
// The two-level lock, bound to core's rules (`beebeeb_api::flows::lock`).
pub mod lock_api;
pub mod lock_password_api;
pub mod owned_state;
// What the app window may change of the rows of the Backups tree, for the native Files window.
pub mod backup_places_api;
pub mod passkeys_api;
pub mod photo_previews_api;
pub mod pins_api;
pub mod photos_api;
pub mod settings_api;
pub mod setup_api;
pub mod speed_test_api;
// Storage use and quota from the server.
mod new_file_api;
mod storage_usage_api;
pub mod support_bundle_api;
// A link to one of the account's files, for the native Files window's Copy link.
pub mod share_link_api;
// What the account shares and what is shared with it, for the native Shared window.
pub mod shares_api;
// Invite someone (DW-0514): share an own item with a person, for the native Shared window.
pub mod share_invite_api;
pub mod signin_api;
pub mod signup_api;
// Is the local state the signed-in account's (task 2488): a read-only check for windows that show it.
pub mod state_owner;
pub mod transfer_limits_api;
pub mod trash_api;
mod thumb_cache_api;
// What uploads, Try again on one, and the cancel of a new file's upload, for the native Files window.
pub mod downloads_api;
pub mod uploads_api;
// A file's versions and the restore of one, for the native Files window.
pub mod trash_bulk_api;
pub mod trash_purge_api;
pub mod versions_api;
// An open file for the native viewer: its facts, and its content decrypted in memory.
pub mod viewer_api;
pub mod office_edit_api;
pub mod office_recovery;
pub mod office_scratch;
pub mod office_session_end;
pub mod viewer_edit_api;

pub use credential_stores::{CredentialStores, PlatformCredentials};
pub use explorer_api::ExplorerChangeRefused;
pub use owned_state::{LocalStateRefused, OwnedState};

/// What the host names once: which account slot, where its local state lives, which API.
pub struct EngineConfig {
    /// The id the credential store segments under (`DesktopConfig::ensure_account_id`).
    pub account_id: String,
    /// The app-local state dir (`state.db` lives here).
    pub state_dir: PathBuf,
    /// The API base, no trailing slash. The sign-in steps AND the running engine talk to this one (the handle
    /// passes it to the runner; nothing started from this handle reads `BB_API_BASE`).
    pub api_base: String,
}

/// What a start came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncStart {
    Started,
    AlreadyRunning,
    /// Nothing is signed in with keys in memory.
    NoSession,
}

/// Why a start made no engine (contract §4.2). Nothing was synced, uploaded, deleted or registered for it. No
/// variant carries a path, a file's name, an email, an id, a key or a token: a window names the folder itself (it
/// knows which one it asked for).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncStartRefused {
    /// Windows has the folder registered as another sync provider's folder.
    FolderOfAnotherProvider,
    /// Windows could not say whose sync folder the folder is (never read as nobody's).
    FolderUnreadable,
    /// The folder is gone while this PC holds the account's local data for it: it is not made again, empty.
    FolderMissing,
    /// Windows refused to register the folder, or to connect it; its code when it gave one.
    RegistrationFailed { code: Option<i32> },
    /// This PC's local data could not be proven the signed-in account's (`account_binding`).
    OtherAccountsData,
    /// w29: the previous account left a file here that may hold changes it never sent: it is never removed unasked.
    /// The only way on is that account's (it sends them) or the person's (the file moves out of the folder).
    UnsentFileOfAnotherAccount,
    /// dc-RC-b: the previous account left files of its own making in the folder (ordinary files, or files changed
    /// since they synced), which a switch never removes unasked. The person keeps them aside or discards them
    /// (`app::account_switch`); until then every sign-in of another account is refused here.
    LeftoverFilesOfAnotherAccount,
    /// Whose this PC's local data is, or whose the session is, could not be confirmed.
    IdentityUnknown,
    /// An earlier engine did not confirm it stopped: only a restart of the app helps.
    StopUnconfirmed,
    /// A stopped ownership proof still holds its client: the refusal ends by itself once its write ends.
    StopFinishing,
    /// A live process holds this app's engine lock (another copy of the app, or `bb watch`).
    StateInUse,
    /// The local state could not be read or written, or an engine ended by itself.
    Other,
}

impl SyncStartRefused {
    /// The refusal's name, for a log line: the kind only, never a path or a sentence with a name in it.
    pub fn kind(self) -> &'static str {
        match self {
            Self::FolderOfAnotherProvider => "folder_of_another_provider",
            Self::FolderUnreadable => "folder_unreadable",
            Self::FolderMissing => "folder_missing",
            Self::RegistrationFailed { .. } => "registration_failed",
            Self::OtherAccountsData => "other_accounts_data",
            Self::UnsentFileOfAnotherAccount => "unsent_file_of_another_account",
            Self::LeftoverFilesOfAnotherAccount => "leftover_files_of_another_account",
            Self::IdentityUnknown => "identity_unknown",
            Self::StopUnconfirmed => "stop_unconfirmed",
            Self::StopFinishing => "stop_finishing",
            Self::StateInUse => "state_in_use",
            Self::Other => "other",
        }
    }

    /// The refusal a binding error is: the two binding sentences by their exact words, anything else `Other`.
    fn of_binding(error: &str) -> Self {
        match account_binding::Refusal::of(error) {
            Some(account_binding::Refusal::OtherAccountOnWindows) => Self::OtherAccountsData,
            Some(account_binding::Refusal::IdentityUnknown) => Self::IdentityUnknown,
            Some(account_binding::Refusal::EngineStopUnconfirmed) => Self::StopUnconfirmed,
            None => Self::Other,
        }
    }
}

impl std::fmt::Display for SyncStartRefused {
    /// The engine's sentence: what happened, and that nothing was changed. A folder is "the sync folder", never a
    /// path and never its name.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FolderOfAnotherProvider => {
                f.write_str("The sync folder is already another app's sync folder. Nothing was changed.")
            }
            Self::FolderUnreadable => f.write_str("The sync folder's owner could not be read. Nothing was changed."),
            Self::FolderMissing => f.write_str(
                "The sync folder is gone. Nothing was changed, and sync does not start in a new, empty folder.",
            ),
            Self::RegistrationFailed { code: Some(code) } => write!(
                f,
                "{} did not register the sync folder (error 0x{:08X}). Nothing was changed.",
                if cfg!(target_os = "macos") { "macOS" } else { "Windows" },
                *code as u32
            ),
            Self::RegistrationFailed { code: None } => {
                write!(
                    f,
                    "{} did not register the sync folder. Nothing was changed.",
                    if cfg!(target_os = "macos") { "macOS" } else { "Windows" }
                )
            }
            Self::OtherAccountsData => f.write_str(account_binding::OTHER_ACCOUNT_DATA),
            Self::UnsentFileOfAnotherAccount => f.write_str(
                "A file left in the sync folder by the account that was signed in before may hold changes that were never sent. Nothing was uploaded or deleted.",
            ),
            Self::LeftoverFilesOfAnotherAccount => f.write_str(
                "The account that was signed in before left files in the sync folder. Nothing was uploaded or deleted.",
            ),
            Self::IdentityUnknown => f.write_str(account_binding::IDENTITY_UNKNOWN),
            Self::StopUnconfirmed => f.write_str(account_binding::ENGINE_STOP_UNCONFIRMED),
            Self::StopFinishing => f.write_str(account_binding::ENGINE_STOP_FINISHING),
            Self::StateInUse => {
                f.write_str("Another program is syncing with this app's local state. Nothing was changed.")
            }
            Self::Other => f.write_str("Sync didn\u{2019}t start. Nothing was changed."),
        }
    }
}

/// The last word of [`Engine::stop`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncStop {
    Stopped,
    WasNotRunning,
    /// This stop or an earlier one did not confirm its engine ended: no new one starts in this process (restart).
    Unconfirmed,
}

/// The handle. Share it as an `Arc`; every method takes `&self`.
pub struct Engine<C: CredentialStores> {
    /// Which app this is: every name the engine touches outside its own memory comes from here.
    identity: &'static EngineIdentity,
    config: EngineConfig,
    host: Arc<dyn EngineHost>,
    stores: C,
    accounts: Accounts,
    acct: Arc<AccountRuntime>,
    runtime: tokio::runtime::Runtime,
    client: reqwest::Client,
    /// The session tokens this process still owes the server a logout for (LD-0028): sign-out records into it, the
    /// engine start and the network coming back pay from it.
    owed: Arc<crate::keychain::owed_revocations::OwedRevocations>,
    /// One session step at a time (a sign-in step, a lock, a sign-out, an engine start or stop).
    transition: tokio::sync::Mutex<()>,
    /// The order of the credential-store writes and the session generation.
    /// Session changes take it for writing; a reader that asks whether its session is still there takes it for
    /// reading without waiting, so readers never refuse each other, only yield to a session change.
    writes: std::sync::RwLock<()>,
    /// A token is stored: someone is signed in, unlocked or not.
    auth_present: Mutex<bool>,
    /// A proven password waiting for its second factor (core holds the partial token, wiped on drop).
    /// The email is wiped with it (the shell's `Pending2fa` does the same).
    pending_2fa: Mutex<Option<(PendingSecondFactor, Zeroizing<String>)>>,
    /// The screen lock's password check state (`lock_password_api`).
    lock_password: lock_password_api::LockPassword,
    /// Wrong passwords in a row, for the lock hint.
    failures: AtomicU32,
    /// [`Engine::watch_session`] ran on this handle.
    session_watch_started: AtomicBool,
    /// What the server last said about the plan, for the No plan gate ([`Engine::gate`]).
    plan: Arc<Mutex<PlanKnown>>,
    /// Items moved to or from Trash through this handle (`trash_api`), for a window that keeps a Trash count.
    trash_moves: AtomicU64,
    /// The two-level lock: core's state machine, moved only by `lock_api`.
    lock_flow: Mutex<beebeeb_api::flows::LockFlow>,
    /// What a window reads without waiting ([`Engine::view`]); `host` writes the engine's reports into it.
    view: Arc<ViewCell>,
    /// The user id the server named for the session: from the profile at a sign-in, from the record beside the stored
    /// session at a restart. Cleared by a sign-out. Read by `owned_state` and the sync start, so an email alone
    /// never decides whose the local state is while the id is known.
    session_user_id: Mutex<Option<String>>,
    /// User ids whose recent searches a session end could not delete; deleted at their next open (`owned_state`).
    recent_searches_due: Mutex<std::collections::BTreeSet<String>>,
    /// Explorer's command: the binding a stop removes, and the folder an engine started in (`explorer_api`).
    explorer: Mutex<explorer_api::ExplorerSeat>,
    /// Test builds only: where the account-bound handle reads (`Engine::use_local_state_for_test`).
    #[cfg(any(test, feature = "test-support"))]
    local_state_for_test: Mutex<Option<PathBuf>>,
    /// Test builds only: run once in the middle of the next lock or sign-out (`Engine::during_next_forget_for_test`).
    #[cfg(any(test, feature = "test-support"))]
    forget_hook_for_test: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

/// The refusal of a sign-out while an open editor holds edits that no save has landed.
pub const UNSAVED_EDITS: &str = "unsaved edits in an open document";

impl<C: CredentialStores> Engine<C> {
    /// Build the handle; a 0.8.x PC's flat-layout session moves into the account's slot. Then [`Engine::restore`].
    pub fn open(
        identity: &'static EngineIdentity,
        config: EngineConfig,
        host: Arc<dyn EngineHost>,
        stores: C,
    ) -> Result<Self, String> {
        // A test build keeps its list in memory, one per handle: the machine's own store is never touched and one
        // test's owed token is never paid to another test's server.
        #[cfg(test)]
        let owed = Arc::new(crate::keychain::owed_revocations::OwedRevocations::with_backend(Box::new(
            crate::keychain::owed_revocations::MemoryOwed::default(),
        )));
        #[cfg(not(test))]
        let owed = Arc::new(crate::keychain::owed_revocations::OwedRevocations::platform(identity));
        let handle = Self::open_owing(identity, config, host, stores, owed.clone())?;
        #[cfg(not(test))]
        session_revoke::install_owed(owed);
        Ok(handle)
    }

    /// [`Engine::open`] with the owed list given: what is owed is paid at once, and again each time the engine
    /// reports the network back.
    pub(crate) fn open_owing(
        identity: &'static EngineIdentity,
        config: EngineConfig,
        host: Arc<dyn EngineHost>,
        stores: C,
        owed: Arc<crate::keychain::owed_revocations::OwedRevocations>,
    ) -> Result<Self, String> {
        if let Some(refusal) = identity.refusal_on(std::env::consts::OS) {
            return Err(refusal.to_string());
        }
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("beebeeb-engine")
            .build()
            .map_err(|_| "could not start the engine's runtime".to_string())?;
        // For this handle's API base, with that base's proxy rule (not the process's `BB_API_BASE`).
        let client = auth_transport::client_for(&config.api_base)?;
        // The running engine reads the process-wide state dir; a handle that names another one would bind one
        // database and sync into another.
        if state_paths::beebeeb_state_dir_for(identity).ok().as_deref() != Some(config.state_dir.as_path()) {
            return Err("the engine handle's state dir is not the process's state dir".to_string());
        }
        let accounts = Accounts::default();
        crate::account::synthesize_single_account(&accounts, AccountId(config.account_id.clone()));
        let acct = accounts.active_account()?;
        // Every engine started from here reports through the view's host, which goes on to the host given here.
        let view = Arc::new(ViewCell::default());
        let host: Arc<dyn EngineHost> = Arc::new(owed_online::OwedPayingHost::new(
            host,
            runtime.handle().clone(),
            client.clone(),
            config.api_base.clone(),
            owed.clone(),
        ));
        let host: Arc<dyn EngineHost> = Arc::new(ViewHost::new(host, view.clone()));
        downloads_api::wake_the_host_for_downloads(&host);
        credential_stores::adopt_legacy_flat(identity, &config.account_id, &stores);
        session_watch::start(&runtime, &acct, &config.api_base);
        {
            // What an earlier sign-out could not tell the server is paid now (and again when the network is back).
            let _enter = runtime.enter();
            session_revoke::pay_owed_detached(client.clone(), config.api_base.clone(), owed.clone());
        }
        Ok(Self {
            identity,
            config,
            host,
            stores,
            accounts,
            acct,
            runtime,
            client,
            owed,
            transition: tokio::sync::Mutex::new(()),
            writes: std::sync::RwLock::new(()),
            auth_present: Mutex::new(false),
            pending_2fa: Mutex::new(None),
            lock_password: lock_password_api::LockPassword::default(),
            failures: AtomicU32::new(0),
            session_watch_started: AtomicBool::new(false),
            plan: Arc::new(Mutex::new(PlanKnown::default())),
            trash_moves: AtomicU64::new(0),
            lock_flow: Mutex::new(beebeeb_api::flows::LockFlow::at_start(lock_api::NO_SESSION)),
            view,
            session_user_id: Mutex::new(None),
            recent_searches_due: Mutex::default(),
            explorer: Mutex::new(explorer_api::ExplorerSeat::new(identity)),
            #[cfg(any(test, feature = "test-support"))]
            local_state_for_test: Mutex::new(None),
            #[cfg(any(test, feature = "test-support"))]
            forget_hook_for_test: Mutex::new(None),
        })
    }

    /// What a window may read of sync without waiting (`crate::sync_view`): a copy, from any thread. It takes no
    /// lock that a transition, the engine slot or the credential store holds; the pause switch and the session's
    /// health are read from their atomics.
    pub fn view(&self) -> SyncView {
        let mut view = self.view.read();
        view.paused = self.acct.sync_paused.load(Ordering::SeqCst);
        view.session_ended = self.acct.auth_health.is_expired();
        view
    }

    /// Test builds only: the view as a start or a stop leaves it, for a test that fills the engine slot itself.
    #[cfg(any(test, feature = "test-support"))]
    pub fn set_view_for_test(&self, change: impl FnOnce(&mut SyncView)) {
        self.view.write(change);
    }

    /// The end of a transition: the view's gate is the gate now, and the window is woken to read it.
    fn publish(&self) {
        self.ask_plan(false);
        let gate = self.gate();
        self.view.write(|view| view.gate = gate);
        self.host.wake();
    }

    fn vault(&self) -> AuthVault<C::Store> {
        AuthVault::new(self.stores.for_account(self.identity, &self.config.account_id))
    }

    fn unlocked(&self) -> bool {
        self.acct
            .session
            .lock()
            .map(|session| session.is_some())
            .unwrap_or(false)
    }

    fn remember_session_user_id(&self, user_id: Option<String>) {
        let user_id = user_id.map(|id| id.trim().to_string()).filter(|id| !id.is_empty());
        *self.session_user_id.lock().unwrap_or_else(|p| p.into_inner()) = user_id;
    }

    fn set_auth_present(&self, present: bool) {
        if let Ok(mut guard) = self.auth_present.lock() {
            *guard = present;
        }
    }

    /// Take this step's turn: the write lock, and a refusal when another session transition happened since `turn`
    /// was captured. On success the generation moves on.
    fn claim(&self, turn: SessionGeneration) -> Result<std::sync::RwLockWriteGuard<'_, ()>, String> {
        let writes = self.writes.write().unwrap_or_else(|p| p.into_inner());
        if !self.acct.session_unchanged_since(turn) {
            return Err(
                "Something changed on this computer while Beebeeb was working; nothing was saved. Try again."
                    .to_string(),
            );
        }
        self.acct
            .advance_session_generation_holding_session_write_lock(SessionTransition::Write);
        Ok(writes)
    }

    /// Session restore at start: a stored token means signed in, and core's start rule (`lock_api`) says how: the
    /// app starts SCREEN-LOCKED with the stored key that is this account's in memory
    /// (`store::load_session_from_vault`, the owner check of task 2447 before the key is read), or with the vault
    /// CLOSED (the start setting, the closed mark, or a flag that cannot be read) and the key entry not read at all.
    /// A key that is not provably this account's is left where it is and not loaded. No network.
    pub fn restore(&self) -> GateState {
        self.runtime.block_on(async {
            let _step = self.transition.lock().await;
            let turn = self.acct.session_generation();
            let vault = self.vault_at_start(); // A clear cut short is finished first (`unfinished_sign_out`).
            let present = matches!(vault.session_token(), Ok(Some(_)));
            self.set_auth_present(present);
            if present {
                // Whose the stored session is, as its sign-in recorded it (unknown stays unknown).
                self.remember_session_user_id(vault.session_owner().ok().flatten());
            }
            if !self.unlocked() {
                let flow = self.lock_start_rule(&vault, present);
                if present && beebeeb_api::flows::key_may_be_loaded(flow.state()) {
                    self.restore_key(turn, vault);
                }
                self.lock_started(flow);
            }
            self.fix_upgrade_candidate_at_startup();
            self.delete_recent_searches_at_signed_out_start(present);
        });
        self.publish();
        self.view.read().gate
    }

    /// Which screen the window shows (`gate::decide_gate`). `NoPlan` only when the server last said the account has
    /// no active plan (none, or lapsed: trial or subscription ended, payment failed past grace). No network here: a
    /// server that could not be reached leaves the last known answer, and no answer at all never blocks.
    pub fn gate(&self) -> GateState {
        let unlocked = self.unlocked();
        if unlocked && self.lock_state() == beebeeb_api::flows::LockState::ScreenLocked {
            return GateState::ScreenLocked;
        }
        let present = self.auth_present.lock().map(|p| *p).unwrap_or(false);
        let stored = if present && !unlocked {
            // A vault marked closed: the key entry is not read, also not to ask whether it is there.
            let vault = self.vault();
            if self.marked_closed(&vault) {
                StoredKey::Usable
            } else {
                key_owner::stored_key_state(&vault)
            }
        } else {
            StoredKey::Absent
        };
        let no_plan = unlocked && self.plan_known_none();
        gate::decide_gate(present || unlocked, unlocked, stored, no_plan)
    }

    /// The window's gate buttons (Try again on the credential-store screen, Check again on the no-plan screen):
    /// read the gate once more and publish it, so `view().gate` changes when the store answers or the plan arrived.
    /// Never the start path: [`Engine::restore`] is not run again.
    pub fn recheck_gate(&self) -> GateState {
        self.ask_plan(true);
        self.publish();
        self.view.read().gate
    }

    /// The signed-in account's email, as the server spelled it.
    pub fn account_email(&self) -> Option<String> {
        self.acct.auth_email.lock().ok().and_then(|email| email.clone())
    }

    /// The signed-in account's user id, once the server named it.
    pub fn user_id(&self) -> Option<String> {
        self.acct
            .cached_profile
            .lock()
            .ok()
            .and_then(|profile| profile.as_ref().map(|profile| profile.user_id.clone()))
    }

    fn problem(&self, code: &str) -> SignInProblem {
        let failures = if code.contains("Invalid email or password") || code == "invalid_credentials" {
            self.failures.fetch_add(1, Ordering::SeqCst) + 1
        } else {
            self.failures.load(Ordering::SeqCst)
        };
        gate::problem_from(code, failures)
    }

    /// Password sign-in (OPAQUE in core's `auth_flow`). `TwoFactor`: call [`Engine::two_factor`] next.
    /// `RecoveryPhrase`: signed in, this computer holds no key of the account. `Drive`: signed in and unlocked.
    pub fn sign_in_password(&self, email: &str, password: Zeroizing<String>) -> Result<NextAfterSignIn, SignInProblem> {
        let result = self.sign_in_password_step(email, password, false);
        self.publish();
        result
    }

    /// `unlocking`: the strong unlock of the account whose session is stored (`lock_api`), not a first sign-in.
    fn sign_in_password_step(
        &self,
        email: &str,
        password: Zeroizing<String>,
        unlocking: bool,
    ) -> Result<NextAfterSignIn, SignInProblem> {
        self.runtime.block_on(async {
            let _step = self.transition.lock().await;
            let turn = self.acct.session_generation();
            if self.auth_present.lock().map(|p| *p).unwrap_or(!unlocking) != unlocking {
                // One account at a time: sign out first (the shell's Windows rule, here on every OS). And only a
                // stored session is unlocked.
                return Err(SignInProblem::Other);
            }
            // A new password step replaces a half-done one: its challenge, stretch and held recovery go first.
            self.forget_half_done_sign_in();
            let email = account_binding::canonical_email(email);
            let legs = password::password_legs(
                |req| auth_transport::send_to(&self.config.api_base, &self.client, None, req),
                &email,
                &password,
            )
            .await
            .map_err(|code| self.problem(&code))?;
            // The server accepted this password: stretch it for the screen lock's check record (`lock_password_api`).
            self.stretch_for_sign_in(&email, &password);
            drop(password);
            match legs {
                PasswordLegs::SecondFactor(pending) => {
                    if !self.acct.session_unchanged_since(turn) {
                        return Err(SignInProblem::Other);
                    }
                    *self.pending_2fa.lock().map_err(|_| SignInProblem::Other)? =
                        Some((pending, Zeroizing::new(email)));
                    Ok(NextAfterSignIn::TwoFactor)
                }
                PasswordLegs::SignedIn(token) => self
                    .finish_sign_in(turn, token, email)
                    .await
                    .map_err(|code| self.problem(&code)),
            }
        })
    }

    /// The second factor of a password sign-in: a 6-digit code or a backup code. A wrong code keeps the challenge.
    /// `Err` is the sentence to show (`gate::two_factor_error`).
    pub fn two_factor(&self, code: Zeroizing<String>) -> Result<NextAfterSignIn, &'static str> {
        let result = self.two_factor_step(code);
        self.publish();
        result
    }

    fn two_factor_step(&self, code: Zeroizing<String>) -> Result<NextAfterSignIn, &'static str> {
        self.runtime.block_on(async {
            let _step = self.transition.lock().await;
            let turn = self.acct.session_generation();
            let words = |code: &str| gate::two_factor_error(code);
            let (request, email) = {
                let pending = self.pending_2fa.lock().map_err(|_| words("failed"))?;
                let Some((challenge, email)) = pending.as_ref() else {
                    return Err(gate::TWO_FACTOR_EXPIRED);
                };
                (
                    password::second_factor_request(challenge, &code).map_err(|e| words(&e))?,
                    email.clone(),
                )
            };
            drop(code);
            let response = auth_transport::send_to(&self.config.api_base, &self.client, None, request)
                .await
                .map_err(|e| words(&e))?;
            let token = {
                let mut pending = self.pending_2fa.lock().map_err(|_| words("failed"))?;
                let Some((challenge, _)) = pending.as_ref() else {
                    return Err(gate::TWO_FACTOR_EXPIRED);
                };
                let token = password::second_factor_answer(challenge, response).map_err(|e| words(&e))?;
                // Accepted: the challenge is spent, and goes (wiped) whatever the install below answers.
                *pending = None;
                token
            };
            self.finish_sign_in(turn, token, (*email).clone())
                .await
                .map_err(|e| words(&e))
        })
    }

    /// A proven sign-in's writes, in its turn: a vault key that is not provably this account's goes first (task
    /// 2447; while it cannot be removed nothing is stored), then the token and the email, then the session's owner
    /// (the id the server named for THIS token). Nothing stored: the new session is revoked.
    async fn finish_sign_in(
        &self,
        turn: SessionGeneration,
        token: Zeroizing<String>,
        typed_email: String,
    ) -> Result<NextAfterSignIn, String> {
        let base = &self.config.api_base;
        // Taken first, so a sign-in that fails below leaves no stretch behind (it is wiped as it drops).
        let screen_check = self.take_pending_screen_check(&typed_email);
        let profile = match fetch_profile(&self.client, base, &token).await {
            Ok(profile) => profile,
            Err(error) => {
                revoke(&self.client, base, &token).await;
                return Err(error);
            }
        };
        let email = match profile.email.trim() {
            "" => typed_email,
            named => named.to_string(),
        };
        // The key this computer kept is the account's only once the server says so; no answer undoes the sign-in.
        let kept = match self.prove_kept_key(&token, &profile.user_id, &email).await {
            Ok(kept) => kept,
            Err(error) => {
                revoke(&self.client, base, &token).await;
                return Err(error);
            }
        };
        // A stored session this sign-in replaces (the strong unlock): it must be this account's, and the key and
        // the engine of the old session leave first (`lock_api`).
        let (turn, replaced) = match self.prepare_replacement(turn, &profile).await {
            Ok(ready) => ready,
            Err(error) => {
                revoke(&self.client, base, &token).await;
                return Err(error);
            }
        };
        let stored = self.store_first_sign_in(turn, &token, &email, &profile);
        if let Err(error) = stored {
            revoke(&self.client, base, &token).await;
            return Err(error);
        }
        self.install_screen_check(screen_check, &profile);
        self.failures.store(0, Ordering::SeqCst);
        let next = self.install_kept_key(&kept, email);
        if let Some(replaced) = replaced.filter(|old| old.as_str() != token.as_str()) {
            // The session the strong unlock replaced is ended at the server, best effort.
            revoke(&self.client, base, &replaced).await;
        }
        Ok(next)
    }

    fn store_first_sign_in(
        &self,
        turn: SessionGeneration,
        token: &str,
        email: &str,
        profile: &AccountProfile,
    ) -> Result<(), String> {
        let _write = self.claim(turn)?;
        let mut vault = self.vault_for_install(&profile.user_id, email)?;
        if let Err(error) = key_owner::remove_foreign_key(&mut vault, &profile.user_id, email) {
            tracing::warn!(%error, "sign-in refused: a stored vault key that is not this account's could not be removed");
            return Err(key_owner::FOREIGN_KEY_NOT_REMOVED.to_string());
        }
        let token = SessionToken::new(token.to_string()).map_err(|e| store::keychain_error("session token", e))?;
        vault
            .install_session(token)
            .map_err(|e| store::keychain_error("store session", e))?;
        if let Err(error) = vault.store_account_email(email) {
            tracing::warn!(%error, "could not store the account email (the session stays unnamed until the next start)");
        }
        if let Err(error) = vault.bind_session_owner(&profile.user_id) {
            tracing::warn!(%error, "could not record the session's owner in the credential store (it stays unknown)");
        }
        if let Ok(mut cached) = self.acct.cached_profile.lock() {
            *cached = Some(profile.clone());
        }
        self.remember_session_user_id(Some(profile.user_id.clone()));
        // A session the server just made is not the one it ended before: the streak of refusals was the earlier
        // session's (the Tauri shell clears it at its sign-out).
        self.acct.auth_health.note_result(None);
        if let Ok(mut stored) = self.acct.auth_email.lock() {
            *stored = Some(email.to_string());
        }
        self.set_auth_present(true);
        self.forget_half_done_sign_in();
        Ok(())
    }

    /// Unlock with the 12 recovery words: the key they derive is proven to be the signed-in account's by the
    /// server's recorded check (`recovery::verify_vault_key_from_phrase`), and only then stored and put in memory.
    /// `Err` is the sentence to show (`gate::phrase_error`).
    pub fn unlock_with_phrase(&self, phrase: Zeroizing<String>) -> Result<(), &'static str> {
        let result = self.unlock_with_phrase_step(phrase);
        self.publish();
        result
    }

    fn unlock_with_phrase_step(&self, phrase: Zeroizing<String>) -> Result<(), &'static str> {
        self.runtime.block_on(async {
            let _step = self.transition.lock().await;
            let turn = self.acct.session_generation();
            if self.unlocked() {
                return Ok(());
            }
            let words = |code: &str| gate::phrase_error(code);
            let vault = self.vault();
            let mut token = match vault.session_token() {
                Ok(Some(token)) => Zeroizing::new(token.expose_for_request().to_string()),
                _ => return Err(words("Sign in before unlocking the vault.")),
            };
            let key = recovery::verify_vault_key_from_phrase(&self.client, &self.config.api_base, &token, &phrase)
                .await
                .map_err(|e| words(&e))?;
            drop(phrase);
            let email = self.account_email();
            {
                let _write = self.claim(turn).map_err(|e| words(&e))?;
                // The token the proof was made for is still the stored one, in this turn.
                let still = vault
                    .session_token()
                    .ok()
                    .flatten()
                    .is_some_and(|stored| stored.expose_for_request() == token.as_str());
                if !still || self.unlocked() {
                    return Err(words("changed"));
                }
                vault
                    .store_wrapped_master_key(SecretBytes::new_master_key(&key))
                    .map_err(|_| words("failed"))?;
                // The key takes the recorded owner of the session it was proven for; an unknown one leaves it untagged.
                if let Err(error) = key_owner::bind_key_to_session_owner(&vault) {
                    tracing::warn!(%error, "could not record the vault key's owner in the credential store");
                }
                let mut guard = self.acct.session.lock().map_err(|_| words("failed"))?;
                *guard = Some(Session {
                    // Moved out of the wiped buffer, not copied: the session holds the one copy from here on.
                    token: std::mem::take(&mut *token),
                    master_key: *key,
                    email,
                });
                drop(guard);
                self.acct.auth_resume_asked.fetch_add(1, Ordering::SeqCst);
                // The twelve words, proven by the server for this session, are a strong proof.
                self.lock_follows(SessionMoved::StrongProof);
            }
            Ok(())
        })
    }

    /// The keys an engine start would use, and whose they are (`start::keys_for_engine_start`). For a host's own
    /// checks and for tests; [`Engine::start_sync`] reads them itself, under the engine slot.
    pub fn keys_for_engine_start(&self) -> Result<Identity, start::EngineStartBlocked> {
        start::keys_for_engine_start(&self.acct).map(|(_keys, identity)| identity)
    }

    /// Start syncing into `sync_root` (contract §4.2): the folder is registered (Windows) and the local data bound to
    /// the session's account before any engine exists, and a refused start changes nothing in the folder and makes
    /// no request. `Ok(NoSession)`: nothing on disk was touched. The view says where the start is
    /// ([`Engine::view`]).
    pub fn start_sync(&self, sync_root: PathBuf) -> Result<SyncStart, SyncStartRefused> {
        self.start_on(sync_root, &OsSyncRoots::new(self.identity))
    }

    /// [`Engine::start_sync`] with the OS's answers from `port`, so a test can make each one on any OS.
    #[cfg(any(test, feature = "test-support"))]
    pub fn start_sync_with(&self, sync_root: PathBuf, port: &dyn SyncRootPort) -> Result<SyncStart, SyncStartRefused> {
        self.start_on(sync_root, port)
    }

    fn start_on(&self, sync_root: PathBuf, port: &dyn SyncRootPort) -> Result<SyncStart, SyncStartRefused> {
        let outcome = self.start_holding_step(sync_root, port);
        if let Err(refused) = outcome {
            // One line, the kind only, in words the log's vocabulary writes as they are (`other accounts data`).
            tracing::warn!(kind = %refused.kind().replace('_', " "), "sync did not start");
            self.view.write(|view| {
                view.start = Start::Refused(refused);
                view.attempt += 1;
                view.explorer = Explorer::Unknown;
            });
            self.wake_own_data_proofs();
        } else if outcome == Ok(SyncStart::NoSession) {
            self.view.write(|view| view.start = Start::NotAsked);
        }
        self.publish();
        outcome
    }

    /// The start's ten steps, in their order, under the transition lock and the engine slot. Steps 1 to 7 only
    /// read, bind `state.db` or refuse; nothing is created, registered or asked of the server before step 8.
    fn start_holding_step(&self, sync_root: PathBuf, port: &dyn SyncRootPort) -> Result<SyncStart, SyncStartRefused> {
        self.runtime.block_on(async {
            let _step = self.transition.lock().await;
            let mut slot = self.acct.engine.lock().await;
            // (1) No engine of an earlier lock is still connected to Cloud Files.
            #[cfg(target_os = "windows")]
            if crate::windows_cf::ensure_reactivation_allowed().is_err() {
                return Err(SyncStartRefused::StopUnconfirmed);
            }
            // (2) One engine at a time. One that ended by itself (its folder could not be connected, its lock was
            // taken) does not run: its end is confirmed and it leaves the slot, so a new start can make one.
            if let Some(runner) = slot.take() {
                if runner.is_running() {
                    *slot = Some(runner);
                    return Ok(SyncStart::AlreadyRunning);
                }
                if unfinished_sign_out::record_the_stop(&self.acct, &runner.abort().await) != SyncStop::Stopped {
                    return Err(SyncStartRefused::StopUnconfirmed);
                }
            }
            // (3) The keys and whose they are, read together. Without a session nothing on disk was touched.
            let (keys, identity) = match start::keys_for_engine_start(&self.acct) {
                Ok(read) => read,
                Err(start::EngineStartBlocked::NoSession) => return Ok(SyncStart::NoSession),
                Err(start::EngineStartBlocked::Refused(sentence)) if sentence == start::ENGINE_START_REFUSED_ERROR => {
                    return Err(SyncStartRefused::StopUnconfirmed);
                }
                Err(start::EngineStartBlocked::Refused(_)) => return Err(SyncStartRefused::Other),
            };
            // The same identity the account-bound handle compares with (task 2488): the id recorded for the session
            // completes an identity that has the email alone.
            let identity = self.with_recorded_user_id(identity);
            if !identity.is_known() {
                return Err(SyncStartRefused::IdentityUnknown);
            }
            self.view.write(|view| {
                view.start = Start::Starting;
                view.explorer = if cfg!(target_os = "windows") {
                    Explorer::Adding
                } else {
                    Explorer::Unknown
                };
            });
            self.host.wake();
            // (4) No other live process syncs with this state, before `state.db` is opened.
            if port.engine_lock_held(&self.config.state_dir) {
                return Err(SyncStartRefused::StateInUse);
            }
            // (4b) A corrupt `state.db` is moved aside (dc-144b): the start makes a fresh one and lists again.
            self.move_aside_a_corrupt_state_db().map_err(|_| SyncStartRefused::Other)?;
            // (5) Never another provider's folder, nor one whose registration cannot be read.
            let registered = port.registration(&sync_root);
            if crate::sync_root_owner::refusal(self.identity, &registered, &sync_root).is_some() {
                return Err(match registered {
                    Registration::Unreadable => SyncStartRefused::FolderUnreadable,
                    _ => SyncStartRefused::FolderOfAnotherProvider,
                });
            }
            // (6) A folder that is gone while the account's data is recorded is not made again, empty.
            let folder_missing = match std::fs::symlink_metadata(&sync_root) {
                Ok(_) => false,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
                Err(_) => return Err(SyncStartRefused::Other),
            };
            if folder_missing && LocalState::holds_account_data(&self.config.state_dir)? {
                return Err(SyncStartRefused::FolderMissing);
            }
            // (7) The local data is bound to the session's account (another account's data refuses, nothing reset).
            let data = self.local_data_for_start(&sync_root, &identity)?;
            // Another account's data goes first.
            let kept_out_by = self.clear_the_previous_account(&data, &identity, &sync_root, port);
            account_binding::bind_before_engine_start(&data, &identity, false).map_err(|error| {
                let refused = match (kept_out_by, SyncStartRefused::of_binding(&error)) {
                    (Some(its_files), SyncStartRefused::OtherAccountsData) => its_files,
                    (_, other) => other,
                };
                self.proof_owed_on(refused, &sync_root)
            })?;
            crate::app::bindings::backup_folders::retag_at_start(self.identity, &identity); // P3-5: re-tag to the id.
            self.settle_an_interrupted_keep(data, &sync_root, &slot);
            // (8) The folder exists.
            crate::config::ensure_directory(&sync_root).map_err(|_| SyncStartRefused::Other)?;
            // (9) Registered as this app's sync root (Windows; nothing to do elsewhere), with Windows' code if not.
            port.register(&sync_root)
                .map_err(|code| SyncStartRefused::RegistrationFailed { code })?;
            self.note_explorer_root(&sync_root);
            // (10) The engine. A restart while paused stays paused: the flag follows the file, under the
            // config-write lock.
            {
                let flag = &self.acct.sync_paused;
                let _ = crate::config::DesktopConfig::path_for(self.identity).and_then(|path| {
                    crate::config::DesktopConfig::update_at_then(
                        &path,
                        |_| (false, ()),
                        |cfg| flag.store(cfg.pause_sync, Ordering::Relaxed),
                    )
                });
            }
            // Before the task exists: its first report must find the view saying it runs, and nothing an earlier
            // engine said is taken for this one's.
            self.view.write(|view| {
                view.start = Start::Running;
                view.attempt += 1;
                view.offline = false;
                view.reported = None;
                view.last_check_at = None;
            });
            let (token, key) = keys;
            *slot = Some(EngineRunner::spawn(
                self.identity,
                self.host.clone(),
                self.config.api_base.clone(),
                sync_root,
                token,
                key,
                self.acct.sync_paused.clone(),
                self.acct.auth_health.clone(),
                ResumeTicket {
                    asked: self.acct.auth_resume_asked.clone(),
                    done: self.acct.auth_resume_done.clone(),
                    pass_wanted: self.acct.reconcile_pass_wanted.clone(),
                    pass_served: self.acct.reconcile_pass_served.clone(),
                },
            ));
            Ok(SyncStart::Started)
        })
    }

    /// Pause sync (the engine keeps running and sends nothing). Written to `desktop.toml`, so a restart stays
    /// paused; the in-memory flag is stored from the file, under the config-write lock.
    pub fn pause(&self) -> Result<(), String> {
        self.set_paused(true)
    }

    pub fn resume(&self) -> Result<(), String> {
        self.set_paused(false)
    }

    fn set_paused(&self, paused: bool) -> Result<(), String> {
        let flag = &self.acct.sync_paused;
        crate::config::DesktopConfig::update_at_then(
            &crate::config::DesktopConfig::path_for(self.identity)?,
            |cfg| {
                cfg.pause_sync = paused;
                (true, ())
            },
            |cfg| flag.store(cfg.pause_sync, Ordering::Relaxed),
        )?;
        self.host
            .event(crate::host::EngineEvent::PauseChanged(crate::host::PauseChanged {
                state: if paused {
                    crate::host::PauseState::Paused
                } else {
                    crate::host::PauseState::Idle
                },
            }));
        Ok(())
    }

    pub fn is_paused(&self) -> bool {
        self.acct.sync_paused.load(Ordering::SeqCst)
    }

    pub fn is_syncing(&self) -> bool {
        self.runtime.block_on(async {
            self.acct
                .engine
                .lock()
                .await
                .as_ref()
                .is_some_and(EngineRunner::is_running)
        })
    }

    /// Stop the engine and wait until its task has ended.
    pub fn stop(&self) -> SyncStop {
        let stopped = self.runtime.block_on(async {
            let _step = self.transition.lock().await;
            self.stop_holding_step().await
        });
        self.publish();
        stopped
    }

    /// The engine's stop, then Explorer's "Free up space" binding is removed (task 2490), whether an engine ran or not
    /// and whether its end was confirmed: the binding outlives a restart. A lock and a sign-out stop through here.
    async fn stop_holding_step(&self) -> SyncStop {
        let stopped = self.stop_runner().await;
        // No pass runs and none is asked for once sync stopped, for this account or the next.
        crate::backup::scheduler::clear_pass_flags();
        self.remove_explorer_command();
        stopped
    }

    async fn stop_runner(&self) -> SyncStop {
        let mut slot = self.acct.engine.lock().await;
        let Some(runner) = slot.take() else {
            return unfinished_sign_out::stop_of_an_empty_slot(&self.acct);
        };
        // The view stops saying sync runs before the task is told to end: the task's own end then changes nothing.
        self.view.write(|view| {
            view.start = Start::NotAsked;
            view.explorer = Explorer::Unknown;
        });
        unfinished_sign_out::record_the_stop(&self.acct, &runner.abort().await)
    }

    /// The vault lock ([`Engine::lock_vault`]): the closed mark, then the engine stops and the keys leave memory
    /// (the stored token and key stay). Every sign-in step that began before this is refused at its write.
    pub fn lock(&self) -> SyncStop {
        self.lock_vault().stopped
    }

    fn forget_session(&self, by: SessionTransition) {
        self.forget_session_as(by, SessionMoved::after(by));
    }

    /// [`Self::forget_session`], with what the lock follows named by the caller.
    fn forget_session_as(&self, by: SessionTransition, moved: SessionMoved) {
        let _writes = self.writes.write().unwrap_or_else(|p| p.into_inner());
        self.acct.advance_session_generation_holding_session_write_lock(by);
        #[cfg(any(test, feature = "test-support"))]
        {
            let hook = self
                .forget_hook_for_test
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .take();
            if let Some(hook) = hook {
                hook();
            }
        }
        let mut guard = self.acct.session.lock().unwrap_or_else(|p| p.into_inner());
        *guard = None;
        drop(guard);
        self.lock_follows(moved);
        self.forget_half_done_sign_in();
    }

    /// Sign out and KEEP the local files and the local state: the engine stops, the session is ended on the server
    /// (best effort), the token, the key and their owners leave the credential store, memory is cleared. Nothing
    /// on disk is deleted but the account's recent searches; the state stays bound to this account, so another
    /// account cannot sync into it.
    /// `Err`: an engine task may still run (its end unconfirmed, now or until a restart), or the store was not cleared.
    pub fn sign_out_keeping_local_data(&self) -> Result<(), String> {
        let result = self.sign_out_keeping_local_data_step();
        self.publish();
        result
    }

    fn sign_out_keeping_local_data_step(&self) -> Result<(), String> {
        self.runtime.block_on(async {
            let _step = self.session_ending_step().await;
            self.sign_out_holding_step().await
        })
    }

    /// The sign-out itself, for a caller that holds the step (`transition`) already: a step that decides to sign
    /// out and does it in the same hold (`signin_api::discard_partial_sign_in`).
    async fn sign_out_holding_step(&self) -> Result<(), String> {
        self.sign_out_holding_step_as(false).await
    }

    /// [`Self::sign_out_holding_step`]. An open editor with edits no save has landed refuses the sign-out (nothing
    /// is stopped, wiped or sent) unless `discard_unsaved`: only the person's explicit Discard drops them.
    async fn sign_out_holding_step_as(&self, discard_unsaved: bool) -> Result<(), String> {
        if !discard_unsaved && self.office_unsaved_count() > 0 {
            return Err(UNSAVED_EDITS.to_string());
        }
        {
            let _ = self.stop_proofs().await; // Not ended in time: its seat refuses starts, the sign-out goes on.
            if unfinished_sign_out::a_task_may_hold_the_keys(&self.acct, self.stop_holding_step().await) {
                return Err(account_binding::ENGINE_STOP_UNCONFIRMED.to_string());
            }
            // Past the gate (a refused sign-out deletes nothing), the session still in memory: its recent searches.
            let _ = self.delete_own_recent_searches_at_session_end(self.acct.session_generation());
            // Every Office recovery blob of this account goes with the session (not a waiting save).
            // The Office host ends its helpers first, so no snapshot lands after the wipe below.
            self.office_before_session_end(office_session_end::SessionEnding::SignOut);
            self.wipe_office_recovery_at_session_end();
            self.end_office_scratch_at_session_end();
            self.wipe_thumb_cache_at_session_end(); // The previous account's thumbnails do not outlive its session.
            let mut vault = self.vault();
            let token = vault
                .session_token()
                .ok()
                .flatten()
                .map(|token| Zeroizing::new(token.expose_for_request().to_string()));
            self.forget_session(SessionTransition::SignOut);
            credential_stores::clear_legacy_flat(self.identity, &self.stores);
            vault
                .clear_session()
                .map_err(|e| store::keychain_error("clear the stored session", e))?;
            if let Ok(mut email) = self.acct.auth_email.lock() {
                *email = None;
            }
            if let Ok(mut profile) = self.acct.cached_profile.lock() {
                *profile = None;
            }
            self.remember_session_user_id(None);
            self.acct.auth_health.note_result(None);
            self.set_auth_present(false);
            if let Some(token) = token {
                // Recorded as owed before this returns; the server is asked by a task of its own (task 1886).
                session_revoke::revoke_detached(
                    self.client.clone(),
                    self.config.api_base.clone(),
                    token.to_string(),
                    self.owed.clone(),
                );
            }
            Ok(())
        }
    }

    /// One page of a folder's rows (`None`: the root), as the Files window lists them: the Backups root first, each
    /// row marked with its place in the Backups tree, a device's manifest hidden (`drive_v2::model::children_of`).
    pub fn listing(
        &self,
        folder_id: Option<&str>,
        sort: Sort,
        offset: usize,
        limit: usize,
    ) -> Result<(Vec<DriveItem>, usize), String> {
        const REFUSED: &str = "could not list this folder";
        self.local_state()
            .map_err(|_| REFUSED.to_string())?
            .with(|db| model::children_of(db, folder_id, sort, offset, limit))
            .map_err(|_| REFUSED.to_string())?
            .map_err(|_| REFUSED.to_string())
    }

    /// Ask the sync engine to take the server's list again, in the signed-in account's local state. Nothing is
    /// asked of a state that is not the session's.
    fn request_resnapshot(&self) {
        if let Ok(state) = self.local_state_for_changes() {
            let _ = state.with(|db| db.request_resnapshot());
        }
    }

    /// The account registry, for a host that needs the runtime's flags (auth health, the engine refusal).
    pub fn accounts(&self) -> &Accounts {
        &self.accounts
    }

    // ── File operations (the native shell's engine_host wires these) ──────────────────────────

    /// The sync root path: the folder sync RUNS in while an engine runs (the one its start noted), else where the
    /// settings name, else the identity's own suggestion. A slot that a start or a stop holds right now counts as not
    /// running, so this never waits behind one; a noted folder whose engine ended by itself is not used.
    pub fn sync_root(&self) -> Result<PathBuf, String> {
        let running = self
            .acct
            .engine
            .try_lock()
            .is_ok_and(|slot| slot.as_ref().is_some_and(crate::runner::EngineRunner::is_running));
        if running && let Some(root) = self.running_root() {
            return Ok(root);
        }
        let config = crate::config::DesktopConfig::load_for(self.identity)?;
        Ok(config
            .sync_root
            .unwrap_or_else(|| crate::config::default_sync_root_suggestion(self.identity)))
    }

    /// The session token (copied for a person's single API call, wiped on drop; `Zeroizing` prints redacted in
    /// `Debug`). `Err` when locked (either level) or signed out.
    fn session_token(&self) -> Result<Zeroizing<String>, String> {
        self.person_session(|session, _| Zeroizing::new(session.token.clone()))
            .map_err(|refused| refused.said_as("locked".to_string(), "not signed in".to_string()))
    }
}

/// The server's record of the account behind `token` (`GET /api/v1/auth/me`). The body is never passed on.
async fn fetch_profile(client: &reqwest::Client, base: &str, token: &str) -> Result<AccountProfile, String> {
    let response = client
        .get(format!("{base}/api/v1/auth/me"))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|_| auth_transport::OFFLINE.to_string())?;
    if !response.status().is_success() {
        return Err("session_not_verified".to_string());
    }
    response
        .json::<AccountProfile>()
        .await
        .map_err(|_| "session_not_verified".to_string())
}

/// The binding's view of this computer's local data, for a start from this handle. It never resets: the binding
/// is asked with resets not allowed, and a reset asked anyway is refused.
///
/// One of the engine's own ways to the state database that do not go through the account-bound handle
/// (`owned_state`): the binding is what DECIDES whose the data is, so it reads before that is known. It hands
/// nothing to a shell.
struct LocalState {
    db: StateDb,
}

impl LocalState {
    /// Whether this PC's local state holds the account's data (step 6 of a start). No `state.db` holds none; one
    /// that cannot be opened or read refuses the start rather than being read as empty.
    fn holds_account_data(state_dir: &std::path::Path) -> Result<bool, SyncStartRefused> {
        let path = state_paths::state_db_path_from_state_dir(state_dir);
        if !path.exists() {
            return Ok(false);
        }
        let db = StateDb::open(&path).map_err(|_| SyncStartRefused::Other)?;
        db.has_account_data().map_err(|_| SyncStartRefused::Other)
    }

    fn for_engine_start(state_dir: &std::path::Path, sync_root: &std::path::Path) -> Result<Self, String> {
        let path = state_paths::state_db_path_from_state_dir(state_dir);
        if !path.exists() {
            state_paths::prepare_beebeeb_state_dir_at(sync_root, state_dir)?;
        }
        let db = StateDb::open(&path).map_err(|e| format!("open state.db: {e}"))?;
        Ok(Self { db })
    }
}

impl account_binding::LocalData for LocalState {
    fn owner(&self) -> Result<Option<Identity>, String> {
        self.db.owner().map_err(|e| format!("read the local data owner: {e}"))
    }

    fn has_account_data(&self) -> Result<bool, String> {
        self.db
            .has_account_data()
            .map_err(|e| format!("inspect the local data: {e}"))
    }

    fn reset(&self) -> Result<(), String> {
        Err(account_binding::OTHER_ACCOUNT_DATA.to_string())
    }

    fn record_owner(&self, owner: &Identity) -> Result<(), String> {
        self.db
            .set_owner(owner)
            .map_err(|e| format!("record the local data owner: {e}"))
    }

    fn adopt_once(&self, candidate: Option<&Identity>) -> Result<bool, String> {
        self.db
            .adopt_owner_once(candidate)
            .map_err(|e| format!("adopt the local data owner: {e}"))
    }
}

#[cfg(test)]
#[path = "sync_start_tests.rs"]
mod sync_start_tests;
