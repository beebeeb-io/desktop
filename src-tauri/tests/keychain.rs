#[path = "../src/keychain.rs"]
mod keychain;

use keychain::{AuthSecretStore, AuthStoreError, AuthVault, SecretBytes, SessionToken, VaultLockState};
use std::sync::Mutex;

#[derive(Default)]
struct MemoryStore {
    session_token: Mutex<Option<SessionToken>>,
    wrapped_master_key: Mutex<Option<Vec<u8>>>,
}

impl AuthSecretStore for MemoryStore {
    fn save_session_token(&self, token: &SessionToken) -> keychain::AuthResult<()> {
        *self.session_token.lock().unwrap() = Some(token.clone());
        Ok(())
    }

    fn load_session_token(&self) -> keychain::AuthResult<Option<SessionToken>> {
        Ok(self.session_token.lock().unwrap().clone())
    }

    fn delete_session_token(&self) -> keychain::AuthResult<()> {
        *self.session_token.lock().unwrap() = None;
        Ok(())
    }

    fn save_wrapped_master_key(&self, wrapped: SecretBytes) -> keychain::AuthResult<()> {
        *self.wrapped_master_key.lock().unwrap() = Some(wrapped.expose_for_crypto().to_vec());
        Ok(())
    }

    fn load_wrapped_master_key(&self) -> keychain::AuthResult<Option<SecretBytes>> {
        self.wrapped_master_key
            .lock()
            .unwrap()
            .clone()
            .map(SecretBytes::new)
            .transpose()
    }

    fn delete_wrapped_master_key(&self) -> keychain::AuthResult<()> {
        *self.wrapped_master_key.lock().unwrap() = None;
        Ok(())
    }
}

#[test]
fn vault_starts_locked_and_fails_closed() {
    let vault = AuthVault::new(MemoryStore::default());

    assert_eq!(vault.lock_state(), VaultLockState::Locked);
    assert!(!vault.can_hydrate_or_upload());
    assert!(matches!(
        vault.master_key(),
        Err(AuthStoreError::InvalidSecret("vault is locked"))
    ));
}

#[test]
fn install_and_clear_session_round_trips_through_store() {
    let mut vault = AuthVault::new(MemoryStore::default());
    let token = SessionToken::new("session-token-for-tests").unwrap();

    vault.install_session(token.clone()).unwrap();
    assert_eq!(
        vault
            .session_token()
            .unwrap()
            .as_ref()
            .map(SessionToken::expose_for_request),
        Some(token.expose_for_request())
    );

    vault
        .store_wrapped_master_key(SecretBytes::new_master_key([7u8; 32]))
        .unwrap();
    vault.unlock().unwrap();
    assert_eq!(vault.lock_state(), VaultLockState::Unlocked);

    vault.clear_session().unwrap();

    assert_eq!(vault.lock_state(), VaultLockState::Locked);
    assert!(vault.session_token().unwrap().is_none());
    assert!(matches!(vault.unlock(), Err(AuthStoreError::NotFound)));
}

#[test]
fn unlock_requires_wrapped_key_and_keeps_key_in_memory_only_while_unlocked() {
    let mut vault = AuthVault::new(MemoryStore::default());

    assert!(matches!(vault.unlock(), Err(AuthStoreError::NotFound)));
    vault
        .store_wrapped_master_key(SecretBytes::new_master_key([42u8; 32]))
        .unwrap();

    vault.unlock().unwrap();
    assert_eq!(vault.lock_state(), VaultLockState::Unlocked);
    assert!(vault.can_hydrate_or_upload());
    assert_eq!(vault.master_key().unwrap(), &[42u8; 32]);

    vault.lock();

    assert_eq!(vault.lock_state(), VaultLockState::Locked);
    assert!(!vault.can_hydrate_or_upload());
}

#[test]
fn unlock_rejects_wrong_sized_master_key_and_remains_locked() {
    let mut vault = AuthVault::new(MemoryStore::default());

    vault
        .store_wrapped_master_key(SecretBytes::new(vec![1, 2, 3]).unwrap())
        .unwrap();

    assert!(matches!(
        vault.unlock(),
        Err(AuthStoreError::InvalidSecret("master key must be 32 bytes"))
    ));
    assert_eq!(vault.lock_state(), VaultLockState::Locked);
    assert!(!vault.can_hydrate_or_upload());
}

#[test]
fn debug_output_redacts_session_token_and_key_material() {
    let token = SessionToken::new("super-secret-session-token").unwrap();
    let secret = SecretBytes::new_master_key([9u8; 32]);

    assert_eq!(format!("{token:?}"), "SessionToken(<redacted>)");
    assert_eq!(format!("{secret:?}"), "SecretBytes(<redacted>, len=32)");
    assert!(!format!("{token:?}").contains("super-secret"));
    assert!(!format!("{secret:?}").contains('9'));
}
