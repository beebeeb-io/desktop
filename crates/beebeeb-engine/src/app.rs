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
//! account; reset or delete local data (a start onto another account's data is refused, on every OS).

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use zeroize::Zeroizing;

use crate::account::{AccountId, AccountRuntime, Accounts, SessionGeneration, SessionTransition};
use crate::account_binding::{self, Identity};
use crate::account_dto::AccountProfile;
use crate::auth_transport;
use crate::drive_v2::dto::DriveItem;
use crate::drive_v2::model::{self, Sort};
use crate::host::EngineHost;
use crate::identity::EngineIdentity;
use crate::keychain::{AuthSecretStore, AuthVault, SecretBytes, SessionToken};
use crate::runner::{EngineRunner, ResumeTicket};
use crate::session::Session;
use crate::signin::gate::{self, GateState, NextAfterSignIn, SignInProblem};
use crate::signin::key_owner::{self, StoredKey};
use crate::signin::password::{self, PasswordLegs};
use crate::signin::{recovery, start, store};
use crate::state_db::StateDb;
use crate::state_paths;
use beebeeb_core::auth_flow::sign_in::PendingSecondFactor;

/// Where the handle keeps an account's session token and vault key. The handle names the app on every call
/// (`identity` is the one [`Engine::open`] was given): a store has no identity of its own to disagree with it.
pub trait CredentialStores: Send + Sync + 'static {
    type Store: AuthSecretStore;
    fn for_account(&self, identity: &'static EngineIdentity, account_id: &str) -> Self::Store;
}

/// The platform's credential store (Windows Credential Manager, the macOS Keychain), per app and account id.
/// It carries no identity: which app's items it reads is the handle's identity, nothing else (review of #309,
/// P2-1: a test handle given the real app's stores was accepted).
pub struct PlatformCredentials;

impl CredentialStores for PlatformCredentials {
    type Store = crate::keychain::PlatformKeychainStore;
    fn for_account(&self, identity: &'static EngineIdentity, account_id: &str) -> Self::Store {
        crate::keychain::platform_keychain_store_for(identity, account_id)
    }
}

#[cfg(any(test, feature = "test-support"))]
impl CredentialStores for crate::signin::memory_store::MemoryCredentials {
    type Store = crate::signin::memory_store::MemoryStore;
    fn for_account(&self, _identity: &'static EngineIdentity, account_id: &str) -> Self::Store {
        self.store_for(account_id)
    }
}

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

/// The last word of [`Engine::stop`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncStop {
    Stopped,
    WasNotRunning,
    /// The engine did not confirm that it ended: no new one starts in this process (restart the app).
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
    /// One session step at a time (a sign-in step, a lock, a sign-out, an engine start or stop).
    transition: tokio::sync::Mutex<()>,
    /// The order of the credential-store writes and the session generation.
    writes: Mutex<()>,
    /// A token is stored: someone is signed in, unlocked or not.
    auth_present: Mutex<bool>,
    /// A proven password waiting for its second factor (core holds the partial token, wiped on drop).
    /// The email is wiped with it (the shell's `Pending2fa` does the same).
    pending_2fa: Mutex<Option<(PendingSecondFactor, Zeroizing<String>)>>,
    /// Wrong passwords in a row, for the lock hint.
    failures: AtomicU32,
}

impl<C: CredentialStores> Engine<C> {
    /// Build the handle. Reads nothing yet: call [`Engine::restore`] next.
    pub fn open(
        identity: &'static EngineIdentity,
        config: EngineConfig,
        host: Arc<dyn EngineHost>,
        stores: C,
    ) -> Result<Self, String> {
        if let Some(refusal) = identity.refusal_on(std::env::consts::OS) {
            return Err(refusal.to_string());
        }
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("beebeeb-engine")
            .build()
            .map_err(|_| "could not start the engine's runtime".to_string())?;
        let client = auth_transport::client()?;
        // The running engine reads the process-wide state dir; a handle that names another one would bind one
        // database and sync into another.
        if state_paths::beebeeb_state_dir_for(identity).ok().as_deref() != Some(config.state_dir.as_path()) {
            return Err("the engine handle's state dir is not the process's state dir".to_string());
        }
        let accounts = Accounts::default();
        crate::account::synthesize_single_account(&accounts, AccountId(config.account_id.clone()));
        let acct = accounts.active_account()?;
        Ok(Self {
            identity,
            config,
            host,
            stores,
            accounts,
            acct,
            runtime,
            client,
            transition: tokio::sync::Mutex::new(()),
            writes: Mutex::new(()),
            auth_present: Mutex::new(false),
            pending_2fa: Mutex::new(None),
            failures: AtomicU32::new(0),
        })
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

    fn set_auth_present(&self, present: bool) {
        if let Ok(mut guard) = self.auth_present.lock() {
            *guard = present;
        }
    }

    /// Take this step's turn: the write lock, and a refusal when another session transition happened since `turn`
    /// was captured. On success the generation moves on.
    fn claim(&self, turn: SessionGeneration) -> Result<std::sync::MutexGuard<'_, ()>, String> {
        let writes = self.writes.lock().unwrap_or_else(|p| p.into_inner());
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

    /// Session restore at start: a stored token means signed in; a stored key that is this account's
    /// (`store::load_session_from_vault`, the owner check of task 2447 before the key is read) unlocks. A key that is
    /// not provably this account's is left where it is and not loaded. No network.
    pub fn restore(&self) -> GateState {
        self.runtime.block_on(async {
            let _step = self.transition.lock().await;
            let turn = self.acct.session_generation();
            let vault = self.vault();
            let present = matches!(vault.session_token(), Ok(Some(_)));
            self.set_auth_present(present);
            if present && !self.unlocked() {
                match store::load_session_from_vault(vault, None) {
                    Ok(Some(session)) => {
                        if let Ok(_write) = self.claim(turn)
                            && let Ok(mut guard) = self.acct.session.lock()
                        {
                            if let Ok(mut email) = self.acct.auth_email.lock() {
                                email.clone_from(&session.email);
                            }
                            *guard = Some(session);
                            self.acct.auth_resume_asked.fetch_add(1, Ordering::SeqCst);
                        }
                    }
                    Ok(None) => {}
                    Err(_) => tracing::info!("startup restore: signed in, the vault stays locked"),
                }
            }
        });
        self.gate()
    }

    /// Which screen the window shows (`gate::decide_gate`). The plan check is not made here: a signed-in, unlocked
    /// account answers `Ready`.
    pub fn gate(&self) -> GateState {
        let unlocked = self.unlocked();
        let present = self.auth_present.lock().map(|p| *p).unwrap_or(false);
        let stored = if (present || unlocked) && !unlocked {
            key_owner::stored_key_state(&self.vault())
        } else {
            StoredKey::Absent
        };
        gate::decide_gate(present || unlocked, unlocked, stored, false)
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
        self.runtime.block_on(async {
            let _step = self.transition.lock().await;
            let turn = self.acct.session_generation();
            if self.auth_present.lock().map(|p| *p).unwrap_or(true) {
                // One account at a time: sign out first (the shell's Windows rule, here on every OS).
                return Err(SignInProblem::Other);
            }
            let email = account_binding::canonical_email(email);
            let legs = password::password_legs(
                |req| auth_transport::send_to(&self.config.api_base, &self.client, None, req),
                &email,
                &password,
            )
            .await
            .map_err(|code| self.problem(&code))?;
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
                let pending = self.pending_2fa.lock().map_err(|_| words("failed"))?;
                let Some((challenge, _)) = pending.as_ref() else {
                    return Err(gate::TWO_FACTOR_EXPIRED);
                };
                password::second_factor_answer(challenge, response).map_err(|e| words(&e))?
            };
            let next = self
                .finish_sign_in(turn, token, (*email).clone())
                .await
                .map_err(|e| words(&e))?;
            if let Ok(mut pending) = self.pending_2fa.lock() {
                *pending = None;
            }
            Ok(next)
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
        let stored = self.store_first_sign_in(turn, &token, &email, &profile);
        if let Err(error) = stored {
            revoke(&self.client, base, &token).await;
            return Err(error);
        }
        self.failures.store(0, Ordering::SeqCst);
        // A key of this same account may still be here (a sign-out that kept it is not this handle's, but an older
        // build's lock is): load it through the guarded loader.
        let turn = self.acct.session_generation();
        if let Ok(Some(session)) = store::load_session_from_vault(self.vault(), Some(email))
            && let Ok(_write) = self.claim(turn)
            && let Ok(mut guard) = self.acct.session.lock()
        {
            *guard = Some(session);
            self.acct.auth_resume_asked.fetch_add(1, Ordering::SeqCst);
        }
        Ok(gate::next_after_sign_in(false, self.unlocked(), false, false))
    }

    fn store_first_sign_in(
        &self,
        turn: SessionGeneration,
        token: &str,
        email: &str,
        profile: &AccountProfile,
    ) -> Result<(), String> {
        let _write = self.claim(turn)?;
        let mut vault = self.vault();
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
        if let Ok(mut stored) = self.acct.auth_email.lock() {
            *stored = Some(email.to_string());
        }
        self.set_auth_present(true);
        Ok(())
    }

    /// Unlock with the 12 recovery words: the key they derive is proven to be the signed-in account's by the
    /// server's recorded check (`recovery::verify_vault_key_from_phrase`), and only then stored and put in memory.
    /// `Err` is the sentence to show (`gate::phrase_error`).
    pub fn unlock_with_phrase(&self, phrase: Zeroizing<String>) -> Result<(), &'static str> {
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
            }
            Ok(())
        })
    }

    /// The keys an engine start would use, and whose they are (`start::keys_for_engine_start`). For a host's own
    /// checks and for tests; [`Engine::start_sync`] reads them itself, under the engine slot.
    pub fn keys_for_engine_start(&self) -> Result<Identity, start::EngineStartBlocked> {
        start::keys_for_engine_start(&self.acct).map(|(_keys, identity)| identity)
    }

    /// Start syncing into `sync_root`. Under the engine slot: the keys and their identity are read together, the
    /// local data is bound to that account (another account's data, or an owner that cannot be compared, refuses
    /// the start and deletes nothing), and only then does an engine exist.
    pub fn start_sync(&self, sync_root: PathBuf) -> Result<SyncStart, String> {
        self.runtime.block_on(async {
            let _step = self.transition.lock().await;
            #[cfg(target_os = "windows")]
            crate::windows_cf::ensure_reactivation_allowed()?;
            crate::sync_root_owner::check(self.identity, &sync_root)?;
            let mut slot = self.acct.engine.lock().await;
            if slot.is_some() {
                return Ok(SyncStart::AlreadyRunning);
            }
            // A restart while paused stays paused: the flag follows the file, under the config-write lock.
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
            let (keys, identity) = match start::keys_for_engine_start(&self.acct) {
                Ok(read) => read,
                Err(start::EngineStartBlocked::NoSession) => return Ok(SyncStart::NoSession),
                Err(refused) => return Err(refused.to_string()),
            };
            if !identity.is_known() {
                return Err(account_binding::IDENTITY_UNKNOWN.to_string());
            }
            let data = LocalState::for_engine_start(&self.config.state_dir, &sync_root)?;
            account_binding::bind_before_engine_start(&data, &identity, false)?;
            drop(data);
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
        self.runtime.block_on(async {
            let _step = self.transition.lock().await;
            self.stop_holding_step().await
        })
    }

    async fn stop_holding_step(&self) -> SyncStop {
        let mut slot = self.acct.engine.lock().await;
        let Some(runner) = slot.take() else {
            return SyncStop::WasNotRunning;
        };
        if runner.abort().await.is_stopped() {
            SyncStop::Stopped
        } else {
            self.acct.engine_stop_unconfirmed.store(true, Ordering::SeqCst);
            SyncStop::Unconfirmed
        }
    }

    /// Lock: stop the engine, then forget the keys in memory (the stored token and key stay). Every sign-in step
    /// that began before this is refused at its write.
    pub fn lock(&self) -> SyncStop {
        self.runtime.block_on(async {
            let _step = self.transition.lock().await;
            let stopped = self.stop_holding_step().await;
            self.forget_session(SessionTransition::Lock);
            stopped
        })
    }

    fn forget_session(&self, by: SessionTransition) {
        let _writes = self.writes.lock().unwrap_or_else(|p| p.into_inner());
        self.acct.advance_session_generation_holding_session_write_lock(by);
        let mut guard = self.acct.session.lock().unwrap_or_else(|p| p.into_inner());
        *guard = None;
        if let Ok(mut pending) = self.pending_2fa.lock() {
            *pending = None;
        }
    }

    /// Sign out and KEEP the local files and the local state: the engine stops, the session is ended on the server
    /// (best effort), the token, the key and their owners leave the credential store, memory is cleared. Nothing
    /// on disk is deleted; the state stays bound to this account, so another account cannot sync into it.
    /// `Err`: the engine did not confirm its stop, or the credential store could not be cleared; still signed in.
    pub fn sign_out_keeping_local_data(&self) -> Result<(), String> {
        self.runtime.block_on(async {
            let _step = self.transition.lock().await;
            if self.stop_holding_step().await == SyncStop::Unconfirmed {
                return Err(account_binding::ENGINE_STOP_UNCONFIRMED.to_string());
            }
            let mut vault = self.vault();
            let token = vault
                .session_token()
                .ok()
                .flatten()
                .map(|token| Zeroizing::new(token.expose_for_request().to_string()));
            self.forget_session(SessionTransition::SignOut);
            vault
                .clear_session()
                .map_err(|e| store::keychain_error("clear the stored session", e))?;
            if let Ok(mut email) = self.acct.auth_email.lock() {
                *email = None;
            }
            if let Ok(mut profile) = self.acct.cached_profile.lock() {
                *profile = None;
            }
            self.set_auth_present(false);
            if let Some(token) = token {
                revoke(&self.client, &self.config.api_base, &token).await;
            }
            Ok(())
        })
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
        let db = self.state_db()?;
        model::children_of(&db, folder_id, sort, offset, limit).map_err(|_| "could not list this folder".to_string())
    }

    /// The local state database, opened read-write as the engine opens it. `Err` before the first engine start
    /// created it.
    pub fn state_db(&self) -> Result<StateDb, String> {
        let path = state_paths::state_db_path_from_state_dir(&self.config.state_dir);
        if !path.exists() {
            return Err("nothing has synced on this computer yet".to_string());
        }
        StateDb::open(&path).map_err(|_| "could not open the local state".to_string())
    }

    /// The account registry, for a host that needs the runtime's flags (auth health, the engine refusal).
    pub fn accounts(&self) -> &Accounts {
        &self.accounts
    }

    // ── File operations (the native shell's engine_host wires these) ──────────────────────────

    /// The sync root path: where the settings name, else the identity's own suggestion.
    pub fn sync_root(&self) -> Result<PathBuf, String> {
        let config = crate::config::DesktopConfig::load_for(self.identity)?;
        Ok(config
            .sync_root
            .unwrap_or_else(|| crate::config::default_sync_root_suggestion(self.identity)))
    }

    /// The session token (cloned for a single API call). `Err` when locked or signed out.
    fn session_token(&self) -> Result<String, String> {
        self.acct
            .session
            .lock()
            .map_err(|_| "session mutex poisoned".to_string())?
            .as_ref()
            .map(|s| s.token.clone())
            .ok_or_else(|| "not signed in".to_string())
    }

    /// Storage use and quota from the server (`/api/v1/billing/usage`).
    pub fn storage_info(&self) -> Result<(i64, i64), String> {
        let token = self.session_token()?;
        self.runtime.block_on(async {
            #[derive(serde::Deserialize)]
            struct Usage {
                used_bytes: i64,
                quota_bytes: i64,
            }
            let resp = self
                .client
                .get(format!("{}/api/v1/billing/usage", self.config.api_base))
                .bearer_auth(&token)
                .send()
                .await
                .map_err(|_| "offline".to_string())?
                .error_for_status()
                .map_err(|_| "could not read storage".to_string())?;
            let usage: Usage = resp.json().await.map_err(|_| "invalid storage response".to_string())?;
            Ok((usage.used_bytes.max(0), usage.quota_bytes.max(0)))
        })
    }

    /// Move items to Trash (server-side, recoverable). Each file id must be known to the server.
    pub fn trash_items(&self, file_ids: &[&str]) -> Result<(), String> {
        if file_ids.is_empty() {
            return Ok(());
        }
        let token = self.session_token()?;
        self.runtime.block_on(async {
            for id in file_ids {
                let resp = self
                    .client
                    .delete(format!("{}/api/v1/files/{id}", self.config.api_base))
                    .bearer_auth(&token)
                    .send()
                    .await
                    .map_err(|_| "offline".to_string())?;
                if !resp.status().is_success() {
                    return Err(format!("could not trash item"));
                }
            }
            // Ask the sync engine to re-snapshot so the listing updates.
            if let Ok(db) = self.state_db() {
                let _ = db.request_resnapshot();
            }
            Ok(())
        })
    }

    /// Restore items from Trash (server-side).
    pub fn restore_items(&self, file_ids: &[&str]) -> Result<(), String> {
        if file_ids.is_empty() {
            return Ok(());
        }
        let token = self.session_token()?;
        self.runtime.block_on(async {
            for id in file_ids {
                let resp = self
                    .client
                    .post(format!("{}/api/v1/files/{id}/restore", self.config.api_base))
                    .bearer_auth(&token)
                    .send()
                    .await
                    .map_err(|_| "offline".to_string())?;
                if !resp.status().is_success() {
                    return Err(format!("could not restore item"));
                }
            }
            if let Ok(db) = self.state_db() {
                let _ = db.request_resnapshot();
            }
            Ok(())
        })
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

/// End `token`'s session on the server. Best effort; never logs.
async fn revoke(client: &reqwest::Client, base: &str, token: &str) {
    let _ = client
        .post(format!("{base}/api/v1/auth/logout"))
        .bearer_auth(token)
        .send()
        .await;
}

/// The binding's view of this computer's local data, for a start from this handle. It never resets: the binding
/// is asked with resets not allowed, and a reset asked anyway is refused.
struct LocalState {
    db: StateDb,
}

impl LocalState {
    fn for_engine_start(state_dir: &std::path::Path, sync_root: &std::path::Path) -> Result<Self, String> {
        let path = state_paths::state_db_path_from_state_dir(state_dir);
        if !path.exists() {
            state_paths::prepare_beebeeb_state_dir_at(sync_root, state_dir)?;
        }
        let db = StateDb::open(&path).map_err(|e| format!("open state.db: {e}"))?;
        db.close_adoption_window().map_err(|e| format!("mark state.db: {e}"))?;
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
        Err(account_binding::OTHER_ACCOUNT_ON_WINDOWS.to_string())
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
