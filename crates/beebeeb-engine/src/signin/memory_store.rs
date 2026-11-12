//! An in-memory credential store for tests (`test` or the `test-support` feature): what the platform store keeps
//! per account, in a map. Clones share the map, so a test can "restart" by opening a new engine handle on a clone.
//!
//! Must not: exist in a release build; be used for anything a person's real session touches.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::keychain::{AuthResult, AuthSecretStore, SecretBytes, SessionToken};

/// Values are wiped when they are replaced, removed or dropped, as a platform store would not leave them.
#[derive(Clone, Default)]
pub struct MemoryCredentials(Arc<Mutex<HashMap<(String, &'static str), zeroize::Zeroizing<Vec<u8>>>>>);

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
}
