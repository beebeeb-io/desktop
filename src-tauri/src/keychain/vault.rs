//! `vault` items split out of `keychain.rs`.

use super::*;

pub trait AuthSecretStore: Send + Sync {
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
}

/// One rule for every "is anything of an account left?" probe: present is present; a store that cannot hold secrets
/// (`Unsupported`) or has none (`NotFound`) holds none; any other answer counts as present, so a caller fails closed.
pub(super) fn presence(answer: AuthResult<bool>) -> bool {
    match answer {
        Ok(present) => present,
        Err(AuthStoreError::Unsupported(_) | AuthStoreError::NotFound) => false,
        Err(_) => true,
    }
}

/// Does this store still hold a session token? Never reads the token where the store can avoid it; fails closed.
pub fn holds_session_token<S: AuthSecretStore>(store: &S) -> bool {
    presence(store.holds_session_token())
}

/// Does this store still hold an account email? Fails closed like [`holds_session_token`].
pub fn holds_account_email<S: AuthSecretStore>(store: &S) -> bool {
    presence(store.holds_account_email())
}

/// Does this store still hold a vault key? It never unlocks and never reads the key where the store can avoid it. A
/// store that cannot hold secrets (`Unsupported`) or has none (`NotFound`) holds none. Any other answer counts as
/// present, so a caller that asks whether anything of an account is left fails closed.
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
    #[cfg(test)]
    pub fn lock_state(&self) -> VaultLockState {
        self.state
    }

    /// Tests only: no production caller reads the store back; it asserts what the store holds.
    #[cfg(test)]
    pub(super) fn store(&self) -> &S {
        &self.store
    }

    pub fn install_session(&self, token: SessionToken) -> AuthResult<()> {
        self.store.save_session_token(&token)
    }

    pub fn session_token(&self) -> AuthResult<Option<SessionToken>> {
        self.store.load_session_token()
    }

    pub fn store_wrapped_master_key(&self, wrapped: SecretBytes) -> AuthResult<()> {
        self.store.save_wrapped_master_key(wrapped)
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
        self.master_key = Some(master_key);
        self.state = VaultLockState::Unlocked;
        Ok(())
    }

    pub fn lock(&mut self) {
        self.master_key.take();
        self.state = VaultLockState::Locked;
    }

    /// Remove only the session token: R8, a token the server rejected, found at startup. The wrapped vault key and the
    /// account email stay; [`Self::clear_session`] remains the full sign-out.
    pub fn clear_session_token(&mut self) -> AuthResult<()> {
        self.store.delete_session_token()
    }

    pub fn clear_session(&mut self) -> AuthResult<()> {
        self.lock();
        self.store.delete_session_token()?;
        self.store.delete_wrapped_master_key()?;
        // Also drop the persisted account email so logout leaves no PII behind.
        // `delete_account_email` is a no-op success when absent, so this stays
        // idempotent and never fails just because email was never stored.
        self.store.delete_account_email()?;
        Ok(())
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
    #[cfg(test)]
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
