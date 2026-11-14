//! An in-memory credential store for tests (`test` or the `test-support` feature): what the platform store keeps
//! per account, in a map. Clones share the map, so a test can "restart" by opening a new engine handle on a clone.
//!
//! Must not: exist in a release build; be used for anything a person's real session touches.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use crate::keychain::{AuthResult, AuthSecretStore, AuthStoreError, LockFlag, SecretBytes, SessionToken};

/// Values are wiped when they are replaced, removed or dropped, as a platform store would not leave them.
#[derive(Clone, Default)]
pub struct MemoryCredentials(
    Arc<Mutex<HashMap<(String, &'static str), zeroize::Zeroizing<Vec<u8>>>>>,
    Arc<Probes>,
);

/// The items of one account in a [`MemoryCredentials`].
#[derive(Clone)]
pub struct MemoryStore {
    all: MemoryCredentials,
    account: String,
}

impl MemoryCredentials {
    pub fn store_for(&self, account_id: &str) -> MemoryStore {
        MemoryStore {
            all: self.clone(),
            account: account_id.to_string(),
        }
    }

    /// How many items are stored, over every account.
    pub fn len(&self) -> usize {
        self.0.lock().expect("memory credentials").len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// From now on no lock flag can be read or written (a credential store that fails), or can again.
    pub fn fail_lock_flags(&self, fail: bool) {
        self.1.fail_lock_flags.store(fail, Ordering::SeqCst);
    }

    /// How often a vault key entry was read, over every account.
    pub fn key_reads(&self) -> usize {
        self.1.key_reads.load(Ordering::SeqCst)
    }
}

/// What a test asks of the store beside its items.
#[derive(Default)]
struct Probes {
    fail_lock_flags: AtomicBool,
    key_reads: AtomicUsize,
}

impl MemoryStore {
    fn save(&self, leaf: &'static str, bytes: &[u8]) -> AuthResult<()> {
        let mut items = self.all.0.lock().expect("memory credentials");
        items.insert((self.account.clone(), leaf), zeroize::Zeroizing::new(bytes.to_vec()));
        Ok(())
    }

    fn load(&self, leaf: &'static str) -> Option<Vec<u8>> {
        let items = self.all.0.lock().expect("memory credentials");
        items.get(&(self.account.clone(), leaf)).map(|bytes| bytes.to_vec())
    }

    fn delete(&self, leaf: &'static str) -> AuthResult<()> {
        let mut items = self.all.0.lock().expect("memory credentials");
        items.remove(&(self.account.clone(), leaf));
        Ok(())
    }

    fn text(&self, leaf: &'static str) -> Option<String> {
        self.load(leaf).and_then(|bytes| String::from_utf8(bytes).ok())
    }
}

impl AuthSecretStore for MemoryStore {
    fn save_session_token(&self, token: &SessionToken) -> AuthResult<()> {
        self.save("session-token", token.expose_for_request().as_bytes())
    }
    fn load_session_token(&self) -> AuthResult<Option<SessionToken>> {
        self.text("session-token").map(SessionToken::new).transpose()
    }
    fn delete_session_token(&self) -> AuthResult<()> {
        self.delete("session-token")
    }
    fn save_wrapped_master_key(&self, wrapped: SecretBytes) -> AuthResult<()> {
        self.save("vault-key", wrapped.expose_for_crypto())
    }
    fn load_wrapped_master_key(&self) -> AuthResult<Option<SecretBytes>> {
        self.all.1.key_reads.fetch_add(1, Ordering::SeqCst);
        self.load("vault-key").map(SecretBytes::new).transpose()
    }
    fn delete_wrapped_master_key(&self) -> AuthResult<()> {
        self.delete("vault-key")
    }
    fn save_account_email(&self, email: &str) -> AuthResult<()> {
        self.save("account-email", email.as_bytes())
    }
    fn load_account_email(&self) -> AuthResult<Option<String>> {
        Ok(self.text("account-email"))
    }
    fn delete_account_email(&self) -> AuthResult<()> {
        self.delete("account-email")
    }
    fn save_vault_key_owner(&self, user_id: &str) -> AuthResult<()> {
        self.save("vault-key-owner", user_id.as_bytes())
    }
    fn load_vault_key_owner(&self) -> AuthResult<Option<String>> {
        Ok(self.text("vault-key-owner"))
    }
    fn delete_vault_key_owner(&self) -> AuthResult<()> {
        self.delete("vault-key-owner")
    }
    fn save_session_owner(&self, user_id: &str) -> AuthResult<()> {
        self.save("session-owner", user_id.as_bytes())
    }
    fn load_session_owner(&self) -> AuthResult<Option<String>> {
        Ok(self.text("session-owner"))
    }
    fn delete_session_owner(&self) -> AuthResult<()> {
        self.delete("session-owner")
    }
    fn save_lock_flag(&self, flag: LockFlag, bytes: &[u8]) -> AuthResult<()> {
        self.lock_flags_answer()?;
        self.save(flag.leaf(), bytes)
    }
    fn load_lock_flag(&self, flag: LockFlag) -> AuthResult<Option<Vec<u8>>> {
        self.lock_flags_answer()?;
        Ok(self.load(flag.leaf()))
    }
    fn delete_lock_flag(&self, flag: LockFlag) -> AuthResult<()> {
        self.lock_flags_answer()?;
        self.delete(flag.leaf())
    }
}

impl MemoryStore {
    fn lock_flags_answer(&self) -> AuthResult<()> {
        if self.all.1.fail_lock_flags.load(Ordering::SeqCst) {
            return Err(AuthStoreError::Backend(
                "the test store refuses the lock flags".to_string(),
            ));
        }
        Ok(())
    }
}
