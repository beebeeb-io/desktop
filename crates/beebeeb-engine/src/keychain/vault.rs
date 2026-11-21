//! `vault` items split out of `keychain.rs`.

use super::*;

/// The flags of the two-level lock that outlive the app (core's `flows::lock::StartFacts`), stored beside the
/// session under core's own entry names, as the bytes `1` / `0`. Not secrets: they say how the app starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockFlag {
    /// "Ask for my password when Beebeeb starts".
    AskPasswordAtStart,
    /// The vault was closed (by hand, by the start setting, or because a flag could not be read).
    VaultClosed,
    /// Not a flag: the screen lock's wrong-try count (`screen_lock::AttemptState::to_bytes`), under core's entry name
    /// `lock.attempts`. Read and written only as bytes ([`AuthVault::lock_entry`]); never logged.
    Attempts,
    /// Not a flag: the screen lock's local check record (`ScreenCheckRecord::to_bytes`), under core's entry name
    /// `lock.screen_check`. Bytes only; never logged.
    ScreenCheck,
    /// "App lock", the screen level's per-device switch (core's `lock.app_lock`). ABSENT IS ON: read it only with
    /// [`AuthVault::lock_setting`]`(.., true)`, never [`AuthVault::lock_flag`]; bytes that are neither value, or a
    /// failed read, count as on.
    AppLock,
    /// Not a flag: "Locks again after", how long the person may be away before an open app locks its screen by
    /// itself while app lock is on (decision 2519: an engine entry beside `lock.app_lock`, `lock.after`). The bytes
    /// are the delay in seconds, one of a closed set (`app::lock_password_api::LockAfter`); read and written only as
    /// bytes ([`AuthVault::lock_entry`]). Absent is the default; bytes that are not one of the set, or a failed read,
    /// count as the strictest choice. Not a secret.
    LockAfter,
}

impl LockFlag {
    /// App lock first: a removal of every entry that stops part way must never leave `lock.app_lock = 0` behind
    /// (it would open the next account's start without a proof). Every removal is tried. An entry left behind by a
    /// removal that failed fails closed, except `lock.after`: a valid delay left behind stays that delay for the
    /// next account (a long one is weaker than the default) until that account's first sign-in removes the entries
    /// again (review of #452, P3-5).
    pub const ALL: [Self; 6] = [
        Self::AppLock,
        Self::AskPasswordAtStart,
        Self::VaultClosed,
        Self::Attempts,
        Self::ScreenCheck,
        Self::LockAfter,
    ];

    pub const fn leaf(self) -> &'static str {
        match self {
            Self::AskPasswordAtStart => "lock.ask_password_at_start",
            Self::VaultClosed => "lock.vault_closed",
            Self::Attempts => "lock.attempts",
            Self::ScreenCheck => "lock.screen_check",
            Self::AppLock => "lock.app_lock",
            Self::LockAfter => "lock.after",
        }
    }
}

const LOCK_FLAGS_UNSUPPORTED: AuthStoreError = AuthStoreError::Unsupported("lock flags are not kept by this store");

pub trait AuthSecretStore: Send + Sync {
    /// Store one lock flag's bytes. A store that keeps none refuses: the lock then lasts for this run only.
    fn save_lock_flag(&self, _flag: LockFlag, _bytes: &[u8]) -> AuthResult<()> {
        Err(LOCK_FLAGS_UNSUPPORTED)
    }
    /// Read one lock flag's bytes; `Ok(None)`: never written. A store that keeps none answers an error, which the
    /// start rule reads as "could not be read": the vault starts closed.
    fn load_lock_flag(&self, _flag: LockFlag) -> AuthResult<Option<Vec<u8>>> {
        Err(LOCK_FLAGS_UNSUPPORTED)
    }
    /// Remove one lock flag. A no-op success when absent.
    fn delete_lock_flag(&self, _flag: LockFlag) -> AuthResult<()> {
        Ok(())
    }
    fn save_session_token(&self, token: &SessionToken) -> AuthResult<()>;
    fn load_session_token(&self) -> AuthResult<Option<SessionToken>>;
    fn delete_session_token(&self) -> AuthResult<()>;
    fn save_wrapped_master_key(&self, wrapped: SecretBytes) -> AuthResult<()>;
    fn load_wrapped_master_key(&self) -> AuthResult<Option<SecretBytes>>;
    fn delete_wrapped_master_key(&self) -> AuthResult<()>;
    /// Persist the signed-in account email (PII metadata) in the credential
    /// vault. Stored as a UTF-8 blob under a dedicated account so it can be
    /// recovered on the auto-unlock path. Default impls below provide
    /// backward-compatible behaviour for any store that predates this entry.
    fn save_account_email(&self, _email: &str) -> AuthResult<()> {
        Err(AuthStoreError::Unsupported(
            "account email storage not supported by this store",
        ))
    }
    /// Read the persisted account email. Returns `Ok(None)` when none was ever
    /// stored (an existing session created before this entry existed), so the
    /// auto-unlock path falls back to `email = None` without erroring.
    fn load_account_email(&self) -> AuthResult<Option<String>> {
        Ok(None)
    }
    /// Remove the persisted account email. A no-op success when absent, matching
    /// the token/key delete semantics so logout stays idempotent.
    fn delete_account_email(&self) -> AuthResult<()> {
        Ok(())
    }
    /// Is a session token stored? A store that can answer without reading the secret does (the real macOS Keychain
    /// asks the item's attributes); the default reads it and drops it at once (`SessionToken` wipes itself).
    fn holds_session_token(&self) -> AuthResult<bool> {
        self.load_session_token().map(|token| token.is_some())
    }
    /// Is a vault key stored? Same contract as [`Self::holds_session_token`] (`SecretBytes` wipes itself).
    fn holds_wrapped_master_key(&self) -> AuthResult<bool> {
        self.load_wrapped_master_key().map(|key| key.is_some())
    }
    /// Is an account email stored? Same contract as [`Self::holds_session_token`].
    fn holds_account_email(&self) -> AuthResult<bool> {
        self.load_account_email().map(|email| email.is_some())
    }

    /// Persist the server user id of the account whose vault key is stored
    /// (task 2447). A sibling credential entry so a loader can refuse a
    /// mismatch. Default returns Unsupported for stores that predate it.
    fn save_vault_key_owner(&self, _user_id: &str) -> AuthResult<()> {
        Err(AuthStoreError::Unsupported(
            "vault key owner storage not supported by this store",
        ))
    }
    /// Read the stored vault key owner. `Ok(None)` = "unknown owner" (legacy
    /// key, pre-task 2447).
    fn load_vault_key_owner(&self) -> AuthResult<Option<String>> {
        Ok(None)
    }
    /// Remove the stored vault key owner. No-op when absent.
    fn delete_vault_key_owner(&self) -> AuthResult<()> {
        Ok(())
    }

    /// Persist the server user id of the account the stored session token belongs to (task 2447).
    fn save_session_owner(&self, _user_id: &str) -> AuthResult<()> {
        Err(AuthStoreError::Unsupported(
            "session owner storage not supported by this store",
        ))
    }
    /// Read the stored session owner. `Ok(None)` = unknown (a session stored before task 2447).
    fn load_session_owner(&self) -> AuthResult<Option<String>> {
        Ok(None)
    }
    /// Remove the stored session owner. No-op when absent.
    fn delete_session_owner(&self) -> AuthResult<()> {
        Ok(())
    }

    /// R4: keep `user_id`'s vault key in that account's own slot (one item per account, named by its server user id,
    /// never by its email), beside the working key above. Written only by [`AuthVault`], and only with a key that was
    /// proven that account's. A store without account slots refuses: the working key is then the only one.
    fn save_account_key(&self, _user_id: &str, _wrapped: SecretBytes) -> AuthResult<()> {
        Err(ACCOUNT_KEYS_UNSUPPORTED)
    }
    /// Read `user_id`'s kept key; `Ok(None)`: none kept (or a store without account slots).
    fn load_account_key(&self, _user_id: &str) -> AuthResult<Option<SecretBytes>> {
        Ok(None)
    }
    /// Remove `user_id`'s kept key only. No-op when absent.
    fn delete_account_key(&self, _user_id: &str) -> AuthResult<()> {
        Ok(())
    }
}

const ACCOUNT_KEYS_UNSUPPORTED: AuthStoreError = AuthStoreError::Unsupported("account key slots are not kept by this store");

/// The same account by server user id; blank is never an account.
fn same_account(a: &str, b: &str) -> bool {
    let (a, b) = (a.trim(), b.trim());
    !a.is_empty() && a == b
}

/// A kept-key step that failed, by its kind only (an error's text can name the item it touched). Never fails the
/// caller: the working key's own rules are unchanged by it.
fn account_slot_refused(step: &'static str, error: &AuthStoreError) {
    if matches!(error, AuthStoreError::Unsupported(_)) {
        return;
    }
    tracing::warn!(
        step,
        kind = match error {
            AuthStoreError::Unsupported(_) => "unsupported",
            AuthStoreError::NotFound => "not_found",
            AuthStoreError::InvalidSecret(_) => "invalid_secret",
            AuthStoreError::Backend(_) => "backend",
        },
        "an account's kept vault key was not written or read"
    );
}

/// One rule for every "is anything of an account left?" probe: present is present; a store that cannot hold secrets
/// (`Unsupported`) or has none (`NotFound`) holds none; any other answer counts as present, so a caller fails closed.
#[cfg(any(any(test, feature = "test-support"), not(target_os = "windows")))]
pub(super) fn presence(answer: AuthResult<bool>) -> bool {
    match answer {
        Ok(present) => present,
        Err(AuthStoreError::Unsupported(_) | AuthStoreError::NotFound) => false,
        Err(_) => true,
    }
}

/// Does this store still hold a session token? Never reads the token where the store can avoid it; fails closed.
#[cfg(any(any(test, feature = "test-support"), not(target_os = "windows")))]
pub fn holds_session_token<S: AuthSecretStore>(store: &S) -> bool {
    presence(store.holds_session_token())
}

/// Does this store still hold an account email? Fails closed like [`holds_session_token`].
#[cfg(any(any(test, feature = "test-support"), not(target_os = "windows")))]
pub fn holds_account_email<S: AuthSecretStore>(store: &S) -> bool {
    presence(store.holds_account_email())
}

/// Does this store still hold a vault key? It never unlocks and never reads the key where the store can avoid it. A
/// store that cannot hold secrets (`Unsupported`) or has none (`NotFound`) holds none. Any other answer counts as
/// present, so a caller that asks whether anything of an account is left fails closed.
#[cfg(any(any(test, feature = "test-support"), not(target_os = "windows")))]
pub fn holds_vault_key<S: AuthSecretStore>(store: &S) -> bool {
    presence(store.holds_wrapped_master_key())
}

pub struct AuthVault<S: AuthSecretStore> {
    store: S,
    state: VaultLockState,
    master_key: Option<SecretBytes>,
}

impl<S: AuthSecretStore> AuthVault<S> {
    pub fn new(store: S) -> Self {
        Self {
            store,
            state: VaultLockState::Locked,
            master_key: None,
        }
    }

    /// Tests only: no production caller reads the state back; it asserts the transitions.
    #[cfg(any(test, feature = "test-support"))]
    pub fn lock_state(&self) -> VaultLockState {
        self.state
    }

    /// Tests only: no production caller reads the store back; it asserts what the store holds.
    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn store(&self) -> &S {
        &self.store
    }

    /// Store a session token. Its owner tag describes the token stored BEFORE, so it is removed first, and a removal
    /// that fails stops the write (task 2447): a new token is never stored next to the id of another session. The
    /// caller binds the new owner afterwards ([`Self::bind_session_owner`]); until then the session's owner is unknown.
    pub fn install_session(&self, token: SessionToken) -> AuthResult<()> {
        self.store.delete_session_owner()?;
        self.store.save_session_token(&token)
    }

    pub fn session_token(&self) -> AuthResult<Option<SessionToken>> {
        self.store.load_session_token()
    }

    /// Store a vault key. Its owner tag describes the key stored BEFORE, so it is removed first, and a removal that
    /// fails stops the write (task 2447): a new key never carries an old owner. The caller binds the new owner
    /// afterwards ([`Self::bind_vault_key_owner`]); a crash in between leaves a key of unknown owner, never a wrong one.
    ///
    /// R4: a working key tagged with an account is first kept in that account's slot (if it has none yet), so the
    /// write never takes another account's only copy.
    pub fn store_wrapped_master_key(&self, wrapped: SecretBytes) -> AuthResult<()> {
        self.keep_tagged_working_key();
        self.store.delete_vault_key_owner()?;
        self.store.save_wrapped_master_key(wrapped)
    }

    /// The working key, read only to copy it into an account's slot. `None`: none, or not a vault key.
    fn working_key(&self) -> AuthResult<Option<SecretBytes>> {
        Ok(self
            .store
            .load_wrapped_master_key()?
            .filter(|key| key.expose_for_crypto().len() == MASTER_KEY_BYTES))
    }

    /// Copy the working key into `user_id`'s slot. The caller has the proof it is that account's (its owner tag).
    fn keep_working_key_for(&self, user_id: &str) -> AuthResult<()> {
        match self.working_key()? {
            Some(key) => self.store.save_account_key(user_id, key),
            None => Ok(()),
        }
    }

    /// Does `user_id` have a kept key? A store without slots, or one that cannot say, answers no.
    fn kept_key(&self, user_id: &str) -> AuthResult<Option<SecretBytes>> {
        Ok(self
            .store
            .load_account_key(user_id)?
            .filter(|key| key.expose_for_crypto().len() == MASTER_KEY_BYTES))
    }

    /// The working key's owner, if it is tagged, gets the working key in its own slot when it has none there yet (the
    /// move of the single slot of an earlier build into the account's slot). Best effort; `true` when that account's
    /// key is kept in its slot afterwards (or the working key has no owner to keep it for).
    fn keep_tagged_working_key(&self) -> bool {
        let owner = match self.vault_key_owner() {
            Ok(Some(owner)) => owner,
            Ok(None) => return true,
            Err(error) => {
                account_slot_refused("read_owner", &error);
                return false;
            }
        };
        match self.kept_key(&owner) {
            Ok(Some(_)) => true,
            Ok(None) => match self.keep_working_key_for(&owner).and_then(|()| self.kept_key(&owner)) {
                Ok(kept) => kept.is_some() || !matches!(self.working_key(), Ok(Some(_))),
                Err(error) => {
                    account_slot_refused("keep", &error);
                    false
                }
            },
            Err(error) => {
                account_slot_refused("read", &error);
                false
            }
        }
    }

    /// R4, at a session the server named `user_id`: the working key becomes that account's kept key, if it has one
    /// and the working key is not already its own. An untagged working key is never replaced (it may be this
    /// account's key, written a moment ago for this very session; its tag follows its proof). Another account's
    /// working key is first kept in its own slot; when that fails, nothing is replaced. The restore writes the key and
    /// then its tag, the order of [`Self::store_wrapped_master_key`]: a stop between them leaves an untagged key.
    fn settle_account_key(&self, user_id: &str) -> AuthResult<()> {
        let present = match self.store.holds_wrapped_master_key() {
            Ok(present) => present,
            Err(AuthStoreError::NotFound | AuthStoreError::Unsupported(_)) => false,
            Err(error) => return Err(error),
        };
        let owner = self.vault_key_owner()?;
        match (present, owner.as_deref()) {
            (true, Some(owner)) if same_account(owner, user_id) => {
                // The single slot of an earlier build, now proven this account's: kept in its slot as well.
                if self.kept_key(user_id)?.is_none() {
                    self.keep_working_key_for(user_id)?;
                }
                return Ok(());
            }
            (true, None) => return Ok(()),
            _ => {}
        }
        let Some(kept) = self.kept_key(user_id)? else {
            // Nothing to restore; another account's working key stays where dc-014 left it, kept in its slot.
            self.keep_tagged_working_key();
            return Ok(());
        };
        if present && !self.keep_tagged_working_key() {
            return Ok(());
        }
        self.store_wrapped_master_key(kept)?;
        self.store.save_vault_key_owner(user_id)
    }

    /// Remove `user_id` from this computer's keys: its kept key, and the working key when it is tagged with that
    /// account. Another account's slot, and an untagged working key, stay. Both removals are tried.
    pub fn forget_account_key(&mut self, user_id: &str) -> AuthResult<()> {
        if user_id.trim().is_empty() {
            return Ok(());
        }
        let kept = self.store.delete_account_key(user_id);
        let working = match self.vault_key_owner() {
            Ok(Some(owner)) if same_account(&owner, user_id) => self.delete_vault_key_and_owner(),
            Ok(_) => Ok(()),
            Err(error) => Err(error),
        };
        kept.and(working)
    }

    /// Is a vault key present? Delegates to the store's `holds_wrapped_master_key`
    /// (the macOS store uses a cheaper attribute-only probe).
    pub fn has_vault_key(&self) -> AuthResult<bool> {
        self.store.holds_wrapped_master_key()
    }

    /// Bind the stored vault key to its owner's server user id (task 2447). Only after the key was proved to be that
    /// account's (the recovery check, a device approval, a browser handoff). A blank id binds nothing.
    pub fn bind_vault_key_owner(&self, user_id: &str) -> AuthResult<()> {
        if user_id.trim().is_empty() {
            return Ok(());
        }
        self.store.save_vault_key_owner(user_id)?;
        // R4: proven that account's, so it is also that account's kept key (a key it rotated replaces the old one).
        if let Err(error) = self.keep_working_key_for(user_id) {
            account_slot_refused("keep", &error);
        }
        Ok(())
    }

    /// Read the vault key owner, or `None` for a key of unknown owner (stored before task 2447, or a write that
    /// stopped between the key and its tag).
    pub fn vault_key_owner(&self) -> AuthResult<Option<String>> {
        match self.store.load_vault_key_owner() {
            Ok(owner) => Ok(owner.filter(|owner| !owner.trim().is_empty())),
            Err(AuthStoreError::Unsupported(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Bind the stored session token to the server user id the server named for it (task 2447). A blank id binds
    /// nothing.
    ///
    /// R4: the session is that account's now, so its kept key (if any) becomes the working key
    /// ([`Self::settle_account_key`]): an account that signs back in after another one unlocked here needs no
    /// twelve words. Best effort: a refusal leaves the working key as it was, and the loader's rules judge it.
    pub fn bind_session_owner(&self, user_id: &str) -> AuthResult<()> {
        if user_id.trim().is_empty() {
            return Ok(());
        }
        self.store.save_session_owner(user_id)?;
        if let Err(error) = self.settle_account_key(user_id) {
            account_slot_refused("restore", &error);
        }
        Ok(())
    }

    /// Read the session owner, or `None` when it is unknown.
    pub fn session_owner(&self) -> AuthResult<Option<String>> {
        match self.store.load_session_owner() {
            Ok(owner) => Ok(owner.filter(|owner| !owner.trim().is_empty())),
            Err(AuthStoreError::Unsupported(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Remove the vault key and its owner tag (task 2447: a key that is not the signing-in account's). The key goes
    /// first and both removals are tried: what a failure leaves is a key with no tag (unknown, never trusted as a
    /// match) or a tag with no key (removed before the next key is stored), never another key under this tag.
    pub fn delete_vault_key_and_owner(&mut self) -> AuthResult<()> {
        self.lock();
        let key = self.store.delete_wrapped_master_key();
        let tag = self.store.delete_vault_key_owner();
        key.and(tag)
    }

    /// Persist the account email alongside the session. Empty/whitespace-only
    /// input is treated as "no email" and skips the write so we never store a
    /// blank credential blob (the backends reject empty blobs anyway).
    pub fn store_account_email(&self, email: &str) -> AuthResult<()> {
        if email.trim().is_empty() {
            return Ok(());
        }
        self.store.save_account_email(email)
    }

    /// Read the persisted account email, or `None` if none was stored (e.g. a
    /// session created before email persistence existed). On a store that does
    /// not support email persistence (the Unsupported error), treat it as
    /// "no email" rather than failing the whole restore.
    pub fn account_email(&self) -> AuthResult<Option<String>> {
        match self.store.load_account_email() {
            Ok(email) => Ok(email),
            Err(AuthStoreError::Unsupported(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn unlock(&mut self) -> AuthResult<()> {
        let Some(master_key) = self.store.load_wrapped_master_key()? else {
            self.lock();
            return Err(AuthStoreError::NotFound);
        };
        if master_key.expose_for_crypto().len() != MASTER_KEY_BYTES {
            self.lock();
            return Err(AuthStoreError::InvalidSecret("master key must be 32 bytes"));
        }
        // R4, the first run after the single slot: a key tagged with its account is kept in that account's slot too.
        // From the key just read (no second read of the store).
        if let Ok(Some(owner)) = self.vault_key_owner() {
            if matches!(self.kept_key(&owner), Ok(None)) {
                // Copied straight into the wiped buffer (the field is this module's).
                let copy = SecretBytes(master_key.0.clone());
                if let Err(error) = self.store.save_account_key(&owner, copy) {
                    account_slot_refused("keep", &error);
                }
            }
        }
        self.master_key = Some(master_key);
        self.state = VaultLockState::Unlocked;
        Ok(())
    }

    pub fn lock(&mut self) {
        self.master_key.take();
        self.state = VaultLockState::Locked;
    }

    /// A stored lock flag, by core's rule (`read_lock_flag`): absent is off; `None` means it could not be read (a
    /// store failure, or bytes that are neither flag value) and the caller treats the vault as closed.
    pub fn lock_flag(&self, flag: LockFlag) -> Option<bool> {
        self.lock_setting(flag, false)
    }

    /// A stored lock setting with what an absent entry means (core's `read_lock_setting`). `None`: it could not be
    /// read (a store failure, or bytes that are neither value); the caller takes the safe side.
    pub fn lock_setting(&self, flag: LockFlag, absent: bool) -> Option<bool> {
        match self.store.load_lock_flag(flag) {
            Ok(None) => Some(absent),
            Ok(Some(bytes)) if bytes == b"1" => Some(true),
            Ok(Some(bytes)) if bytes == b"0" => Some(false),
            Ok(Some(_)) | Err(_) => None,
        }
    }

    /// Remove every lock entry, app lock first ([`LockFlag::ALL`]'s order); every removal is tried. An account's
    /// first sign-in on this computer carries nothing of a lock setting (core's first `Session::save`).
    pub fn clear_lock_entries(&self) -> AuthResult<()> {
        LockFlag::ALL
            .into_iter()
            .fold(Ok(()), |done, flag| done.and(self.store.delete_lock_flag(flag)))
    }

    /// The raw bytes of a lock entry that is not a 1/0 flag ([`LockFlag::Attempts`], [`LockFlag::ScreenCheck`],
    /// [`LockFlag::LockAfter`]).
    pub fn lock_entry(&self, entry: LockFlag) -> AuthResult<Option<Vec<u8>>> {
        self.store.load_lock_flag(entry)
    }

    /// Store the raw bytes of such an entry.
    pub fn set_lock_entry(&self, entry: LockFlag, bytes: &[u8]) -> AuthResult<()> {
        self.store.save_lock_flag(entry, bytes)
    }

    /// Remove a lock flag (absent reads as off).
    pub fn clear_lock_flag(&self, flag: LockFlag) -> AuthResult<()> {
        self.store.delete_lock_flag(flag)
    }

    pub fn set_lock_flag(&self, flag: LockFlag, on: bool) -> AuthResult<()> {
        self.store.save_lock_flag(flag, if on { b"1" } else { b"0" })
    }

    /// Remove only the session token: R8, a token the server rejected, found at startup. The wrapped vault key and the
    /// account email stay; [`Self::clear_session`] remains the full sign-out.
    pub fn clear_session_token(&mut self) -> AuthResult<()> {
        let token = self.store.delete_session_token();
        // The owner tag describes the token just removed (task 2447). Tried even when the token's removal failed.
        let owner = self.store.delete_session_owner();
        token.and(owner)
    }

    /// The full sign-out. Every removal is tried, and the first failure is the result (task 2447): a token that
    /// cannot be removed must not leave the vault key, the email or the owner tags behind as well.
    ///
    /// R4 (Guus, 2026-11-21): signing out of an account removes that account's kept key too, so a sign-out still leaves
    /// nothing of it. The account signing out is the session's owner (or, when that is unknown, the working key's).
    /// Another account's working key (one that was switched away from, still signed in on this PC) goes from the
    /// working slot but is kept in its own slot first. A switch never calls this.
    pub fn clear_session(&mut self) -> AuthResult<()> {
        self.lock();
        let session_owner_id = self.session_owner();
        let key_owner_id = self.vault_key_owner();
        let leaving = match (&session_owner_id, &key_owner_id) {
            (Ok(Some(owner)), _) => Some(owner.clone()),
            (Ok(None), Ok(Some(owner))) => Some(owner.clone()),
            _ => None,
        };
        let others_key = matches!(&key_owner_id, Ok(Some(owner)) if !leaving.as_deref().is_some_and(|l| same_account(l, owner)));
        if others_key {
            self.keep_tagged_working_key();
        }
        // App lock goes FIRST (core's device sign-out): a sign-out cut short must never leave `lock.app_lock = 0`
        // for the next account. Tried again with the other lock entries below.
        let app_lock = self.store.delete_lock_flag(LockFlag::AppLock);
        let token = self.store.delete_session_token();
        let key = self.store.delete_wrapped_master_key();
        // Also drop the persisted account email so logout leaves no PII behind.
        // `delete_account_email` is a no-op success when absent, so this stays
        // idempotent and never fails just because email was never stored.
        let email = self.store.delete_account_email();
        let key_owner = self.store.delete_vault_key_owner();
        let session_owner = self.store.delete_session_owner();
        // The lock's flags belong to the account on this computer: they leave with its session (core removes them
        // at the same place).
        let flags = self.clear_lock_entries();
        // The signing-out account's kept key. An owner that could not be read is a sign-out that cannot name what it
        // left: it fails like any other removal.
        let kept = match leaving {
            Some(owner) => self.store.delete_account_key(&owner),
            None => session_owner_id.and(key_owner_id).map(|_| ()),
        };
        app_lock
            .and(token)
            .and(key)
            .and(email)
            .and(key_owner)
            .and(session_owner)
            .and(flags)
            .and(kept)
    }

    pub fn master_key(&self) -> AuthResult<&[u8]> {
        if self.state != VaultLockState::Unlocked {
            return Err(AuthStoreError::InvalidSecret("vault is locked"));
        }
        self.master_key
            .as_ref()
            .map(SecretBytes::expose_for_crypto)
            .ok_or(AuthStoreError::InvalidSecret("vault is locked"))
    }

    /// Tests only: no production caller asks (the engine reads the master key
    /// itself), and only the auth facade's contract test (`tests/keychain.rs`,
    /// which includes this file whole) drives it.
    #[cfg(any(test, feature = "test-support"))]
    #[allow(dead_code)]
    pub fn can_hydrate_or_upload(&self) -> bool {
        self.master_key().is_ok()
    }
}

impl<S: AuthSecretStore> fmt::Debug for AuthVault<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthVault")
            .field("state", &self.state)
            .field("master_key", &self.master_key)
            .finish_non_exhaustive()
    }
}
