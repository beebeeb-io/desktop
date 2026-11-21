//! Where the handle keeps an account's session token and vault key, and the one older layout it still reads: the
//! flat layout of a 0.8.x PC from before the per-account slots (`io.beebeeb.app/<leaf>`, no account id in the name).
//!
//! At open ([`adopt_legacy_flat`]) a session that still sits in the flat layout moves into the account's slot
//! (`keychain::migrate_legacy_keychain_between`: written, read back, and only then removed from the flat layout).
//! At every clear of the stored session ([`clear_legacy_flat`]) the flat layout is cleared as well: after a move whose
//! read-back failed, the flat items are kept AND the new token may be written, and a clear that skipped them would
//! let the next open's move put the old session back.
//!
//! No read fallback (a contract choice; reverse it only through the lead). The Tauri shell read the flat store when
//! the migration failed; this engine does not. The engine signs in, locks, re-authenticates and signs out on the
//! account's slot only: a session that lived only in a store the engine never writes could not be cleared or replaced
//! correctly. What a failed migration leaves: the person opens signed out (or, when the token was written and its key
//! was not, asked for the twelve words), signs in once, and their local data (kept on Windows) is bound again by R10
//! for the same account. The next open's migration short-circuits (new token and key present) and sweeps the flat
//! items.
//!
//! Two cases where the flat session is never moved, and is deleted instead (the move writes nothing into the slot):
//! - A sign-in of this engine is stored in the slot (a token with its owner tag; CP1 row S1, lead ruling 2026-11-15
//!   18:34: "once a native sign-in exists, a legacy migration retry must never replace it, and the legacy copy is
//!   discarded"). Without this, a move that failed before its token write, then a password sign-in and a quit before
//!   the twelve words, let the next open's move take its "partial write" path and write the flat session over it.
//! - The flat layout is retired (decision D-B3-1, lead 2026-11-15 20:16). Every clear of the stored session writes
//!   the retired mark into the flat layout BEFORE it tries to remove the flat items, so a clear that fails still keeps
//!   the next open from moving the old session back in; the sign-out still returns what it returned.
//!
//! The mark is the flat layout's session-owner item holding [`FLAT_LAYOUT_RETIRED`] ([`mark_retired`],
//! [`is_retired`]). It is the one write this engine makes into the flat layout (`keychain::legacy_platform_keychain_store`
//! documents that store as read-or-delete-only; this is a deviation from that note, flagged for the lead). The mark is
//! removed with the flat items, and only once all of them are gone: flat items that a clear left behind keep it.
//!
//! Must not: log a token, a key, an email or a store error's text (a platform error can name the item it touched);
//! fail an open, or change what a sign-out returns; write a session into the flat layout.

use std::sync::Arc;

use crate::identity::EngineIdentity;
use crate::keychain::{self, AuthResult, AuthSecretStore, AuthStoreError, LockFlag, SecretBytes, SessionToken};

/// Where the handle keeps an account's session token and vault key. The handle names the app on every call
/// (`identity` is the one [`Engine::open`](super::Engine::open) was given): a store has no identity of its own to
/// disagree with it.
pub trait CredentialStores: Send + Sync + 'static {
    type Store: AuthSecretStore;
    fn for_account(&self, identity: &'static EngineIdentity, account_id: &str) -> Self::Store;
    /// The flat layout of a 0.8.x PC: read only to move its session into an account's slot, and cleared at every
    /// clear. `None`: these stores never had that layout.
    fn legacy_flat(&self, _identity: &'static EngineIdentity) -> Option<Self::Store> {
        None
    }
}

/// The platform's credential store (Windows Credential Manager, the macOS Keychain), per app and account id.
/// It carries no identity: which app's items it reads is the handle's identity, nothing else (review of #309,
/// P2-1: a test handle given the real app's stores was accepted).
pub struct PlatformCredentials;

impl CredentialStores for PlatformCredentials {
    type Store = AccountSlots<crate::keychain::PlatformKeychainStore>;
    fn for_account(&self, identity: &'static EngineIdentity, account_id: &str) -> Self::Store {
        AccountSlots::new(
            crate::keychain::platform_keychain_store_for(identity, account_id),
            move |slot| crate::keychain::platform_keychain_store_for(identity, slot),
        )
    }
    /// Windows and macOS had the flat layout. Elsewhere the platform store is a fail-closed stub: asking it would log
    /// a warning at every open for nothing.
    fn legacy_flat(&self, identity: &'static EngineIdentity) -> Option<Self::Store> {
        if cfg!(any(target_os = "windows", target_os = "macos")) {
            Some(AccountSlots::without_account_keys(crate::keychain::legacy_platform_keychain_store(identity)))
        } else {
            None
        }
    }
}

/// The test stores' flat layout: a slot no account id can be. Account ids are minted by
/// `DesktopConfig::ensure_account_id_at` as a v4 UUID in its hyphenated form (hex digits and `-`); this one has
/// spaces and angle brackets.
#[cfg(any(test, feature = "test-support"))]
pub const LEGACY_FLAT_SLOT: &str = "<flat layout>";

#[cfg(any(test, feature = "test-support"))]
impl CredentialStores for crate::signin::memory_store::MemoryCredentials {
    type Store = AccountSlots<crate::signin::memory_store::MemoryStore>;
    fn for_account(&self, _identity: &'static EngineIdentity, account_id: &str) -> Self::Store {
        let all = self.clone();
        AccountSlots::new(self.store_for(account_id), move |slot| all.store_for(slot))
    }
    fn legacy_flat(&self, _identity: &'static EngineIdentity) -> Option<Self::Store> {
        Some(AccountSlots::without_account_keys(self.store_for(LEGACY_FLAT_SLOT)))
    }
}

/// R4: the store of one install slot, plus one kept-key slot per ACCOUNT. Before this, the credential store held one
/// vault key per device, so after a switch A -> B, B's unlock wrote over A's key and A had to type the twelve words
/// again. Every entry of the install slot is read and written as before; [`AuthSecretStore::save_account_key`] and its
/// siblings reach the account's own slot, one item per account: `account-key-<server user id>/wrapped-master-key`.
/// The id is the server's (never the email); one that is not an id refuses. Which key goes there, and when, is
/// [`keychain::AuthVault`]'s rule.
pub struct AccountSlots<S> {
    slot: S,
    account_key_slot: Option<Arc<dyn Fn(&str) -> S + Send + Sync>>,
}

impl<S> AccountSlots<S> {
    pub fn new(slot: S, account_key_slot: impl Fn(&str) -> S + Send + Sync + 'static) -> Self {
        Self {
            slot,
            account_key_slot: Some(Arc::new(account_key_slot)),
        }
    }

    /// The flat layout of a 0.8.x PC: never an account's key slot.
    pub fn without_account_keys(slot: S) -> Self {
        Self {
            slot,
            account_key_slot: None,
        }
    }

    /// The install slot, as a test reads and seeds it.
    #[cfg(any(test, feature = "test-support"))]
    pub fn install_slot(&self) -> &S {
        &self.slot
    }

    /// `user_id`'s key slot. `Unsupported` without slots; `InvalidSecret` for an id that is not a server user id.
    fn account_key_store(&self, user_id: &str) -> AuthResult<S> {
        let Some(make) = self.account_key_slot.as_ref() else {
            return Err(AuthStoreError::Unsupported("account key slots are not kept by this store"));
        };
        Ok(make(&account_key_slot_name(user_id)?))
    }
}

/// The slot name of `user_id`'s kept key. Server user ids are UUIDs: only ASCII letters, digits and `-` pass, so no
/// id can name an install slot (a bare UUID), the flat layout, or a path. Not a secret, but never logged.
pub fn account_key_slot_name(user_id: &str) -> AuthResult<String> {
    let id = user_id.trim();
    if id.is_empty() || id.len() > 64 || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
        return Err(AuthStoreError::InvalidSecret("not an account id"));
    }
    Ok(format!("account-key-{id}"))
}

impl<S: AuthSecretStore> AuthSecretStore for AccountSlots<S> {
    fn save_lock_flag(&self, flag: LockFlag, bytes: &[u8]) -> AuthResult<()> {
        self.slot.save_lock_flag(flag, bytes)
    }
    fn load_lock_flag(&self, flag: LockFlag) -> AuthResult<Option<Vec<u8>>> {
        self.slot.load_lock_flag(flag)
    }
    fn delete_lock_flag(&self, flag: LockFlag) -> AuthResult<()> {
        self.slot.delete_lock_flag(flag)
    }
    fn save_session_token(&self, token: &SessionToken) -> AuthResult<()> {
        self.slot.save_session_token(token)
    }
    fn load_session_token(&self) -> AuthResult<Option<SessionToken>> {
        self.slot.load_session_token()
    }
    fn delete_session_token(&self) -> AuthResult<()> {
        self.slot.delete_session_token()
    }
    fn save_wrapped_master_key(&self, wrapped: SecretBytes) -> AuthResult<()> {
        self.slot.save_wrapped_master_key(wrapped)
    }
    fn load_wrapped_master_key(&self) -> AuthResult<Option<SecretBytes>> {
        self.slot.load_wrapped_master_key()
    }
    fn delete_wrapped_master_key(&self) -> AuthResult<()> {
        self.slot.delete_wrapped_master_key()
    }
    fn save_account_email(&self, email: &str) -> AuthResult<()> {
        self.slot.save_account_email(email)
    }
    fn load_account_email(&self) -> AuthResult<Option<String>> {
        self.slot.load_account_email()
    }
    fn delete_account_email(&self) -> AuthResult<()> {
        self.slot.delete_account_email()
    }
    fn holds_session_token(&self) -> AuthResult<bool> {
        self.slot.holds_session_token()
    }
    fn holds_wrapped_master_key(&self) -> AuthResult<bool> {
        self.slot.holds_wrapped_master_key()
    }
    fn holds_account_email(&self) -> AuthResult<bool> {
        self.slot.holds_account_email()
    }
    fn save_vault_key_owner(&self, user_id: &str) -> AuthResult<()> {
        self.slot.save_vault_key_owner(user_id)
    }
    fn load_vault_key_owner(&self) -> AuthResult<Option<String>> {
        self.slot.load_vault_key_owner()
    }
    fn delete_vault_key_owner(&self) -> AuthResult<()> {
        self.slot.delete_vault_key_owner()
    }
    fn save_session_owner(&self, user_id: &str) -> AuthResult<()> {
        self.slot.save_session_owner(user_id)
    }
    fn load_session_owner(&self) -> AuthResult<Option<String>> {
        self.slot.load_session_owner()
    }
    fn delete_session_owner(&self) -> AuthResult<()> {
        self.slot.delete_session_owner()
    }
    fn save_account_key(&self, user_id: &str, wrapped: SecretBytes) -> AuthResult<()> {
        self.account_key_store(user_id)?.save_wrapped_master_key(wrapped)
    }
    fn load_account_key(&self, user_id: &str) -> AuthResult<Option<SecretBytes>> {
        match self.account_key_store(user_id) {
            Ok(store) => store.load_wrapped_master_key(),
            Err(AuthStoreError::Unsupported(_)) => Ok(None),
            Err(error) => Err(error),
        }
    }
    fn delete_account_key(&self, user_id: &str) -> AuthResult<()> {
        match self.account_key_store(user_id) {
            Ok(store) => store.delete_wrapped_master_key(),
            Err(AuthStoreError::Unsupported(_)) => Ok(()),
            Err(error) => Err(error),
        }
    }
}

/// At open, before anything reads the account's slot: a session the flat layout still holds moves into it. Only a
/// flat layout that holds a session token is moved; without one there is nothing to move, and the account's slot is
/// not read (a closed vault's key entry is read by nothing at a start). Beside a sign-in of this engine, or once the
/// flat layout is retired, the flat session is deleted and nothing is written into the slot ([`must_not_move`]). A
/// refusal of the store is logged by its kind and never fails the open: the flat items then stay where they are, and
/// the next open tries again (and still never writes over the slot).
pub(crate) fn adopt_legacy_flat<C: CredentialStores>(identity: &'static EngineIdentity, account_id: &str, stores: &C) {
    let Some(legacy) = stores.legacy_flat(identity) else {
        return;
    };
    if matches!(legacy.holds_session_token(), Ok(false)) {
        return;
    }
    let slot = stores.for_account(identity, account_id);
    match must_not_move(&legacy, &slot) {
        Ok(false) => {}
        Ok(true) => {
            match discard(&legacy) {
                Ok(()) => tracing::info!(step = "discard", "the old credential layout was discarded"),
                Err(error) => refused(Step::Discard, &error),
            }
            return;
        }
        // The store could not say: nothing is moved and nothing is deleted.
        Err(error) => {
            refused(Step::Move, &error);
            return;
        }
    }
    if let Err(error) = keychain::migrate_legacy_keychain_between(&slot, &legacy) {
        refused(Step::Move, &error);
    }
}

/// At every clear of the stored session, before the account's slot is cleared: the flat layout's items go too, so no
/// later open can move an old session back in. The retired mark goes in first (D-B3-1), so a clear that fails still
/// keeps the next open from moving them. A refusal is logged by its kind and never changes what the caller returns.
pub(crate) fn clear_legacy_flat<C: CredentialStores>(identity: &'static EngineIdentity, stores: &C) {
    let Some(legacy) = stores.legacy_flat(identity) else {
        return;
    };
    if let Err(error) = mark_retired(&legacy) {
        refused(Step::Mark, &error);
    }
    if let Err(error) = discard(&legacy) {
        refused(Step::Clear, &error);
    }
}

/// The retired mark: the flat layout's session-owner item, in words no server user id can be (ids are UUIDs). No
/// 0.8.x build wrote that item into the flat layout, and the move never reads it. Not a secret.
const FLAT_LAYOUT_RETIRED: &str = "flat layout retired";

/// Mark the flat layout retired: from now on its session is deleted at open, never moved.
fn mark_retired<S: AuthSecretStore>(legacy: &S) -> AuthResult<()> {
    legacy.save_session_owner(FLAT_LAYOUT_RETIRED)
}

/// Is the flat layout retired? `Err`: the store could not say.
fn is_retired<S: AuthSecretStore>(legacy: &S) -> AuthResult<bool> {
    Ok(legacy.load_session_owner()?.as_deref() == Some(FLAT_LAYOUT_RETIRED))
}

/// Is a sign-in of this engine stored in the account's slot? A token with its owner tag: what a password sign-in
/// writes (`Engine::store_first_sign_in`), and what the move never leaves (its step 4 removes the slot's owner tags
/// before it writes the flat token, so a move cut short leaves a token without one: its "partial write" case, which
/// keeps its own path). The slot's key is not read.
fn holds_native_sign_in<S: AuthSecretStore>(slot: &S) -> AuthResult<bool> {
    if !slot.holds_session_token()? {
        return Ok(false);
    }
    Ok(slot.load_session_owner()?.is_some_and(|owner| !owner.trim().is_empty()))
}

/// The flat session is never moved once the flat layout is retired, or beside a sign-in of this engine.
fn must_not_move<S: AuthSecretStore>(legacy: &S, slot: &S) -> AuthResult<bool> {
    if is_retired(legacy)? {
        return Ok(true);
    }
    holds_native_sign_in(slot)
}

/// Delete the flat session: its token, key and email first, then the retired mark, only once those are gone (flat
/// items a removal left behind keep the mark).
fn discard<S: AuthSecretStore>(legacy: &S) -> AuthResult<()> {
    keychain::sweep_legacy(legacy)?;
    legacy.delete_session_owner()
}

#[derive(Clone, Copy)]
enum Step {
    Move,
    Clear,
    /// The retired mark was not written.
    Mark,
    /// The flat session was to be deleted instead of moved, and was not.
    Discard,
}

/// A refusal of a step on the flat layout, by its kind only: a platform error's text can name the item it touched.
/// The kinds are words of the log call itself, so the log file writes them as they are.
fn refused(step: Step, error: &AuthStoreError) {
    tracing::warn!(
        step = match step {
            Step::Move => "move",
            Step::Clear => "clear",
            Step::Mark => "mark",
            Step::Discard => "discard",
        },
        kind = match error {
            AuthStoreError::Unsupported(_) => "unsupported",
            AuthStoreError::NotFound => "not_found",
            AuthStoreError::InvalidSecret(_) => "invalid_secret",
            AuthStoreError::Backend(_) => "backend",
        },
        "the old credential layout was not moved or cleared"
    );
}

#[cfg(test)]
#[path = "credential_stores_tests.rs"]
mod tests;
