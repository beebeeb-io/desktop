//! The keychain's `Debug` output names no secret, and a vault starts locked and fails closed. Ported from the old
//! shell's facade test of this module (`tests/keychain.rs`), which was the only test of the redaction.
//!
//! Self-contained on purpose: `keychain.rs` is also compiled by `#[path]` from outside this crate, so this file names
//! nothing beyond the keychain module (`super::*`) and keeps its own small store.

use super::*;
use std::sync::Mutex;

/// The smallest store a vault needs: a session token and a vault key, in memory.
#[derive(Default)]
struct MemoryStore {
    session_token: Mutex<Option<SessionToken>>,
    wrapped_master_key: Mutex<Option<Vec<u8>>>,
}

impl MemoryStore {
    /// A store that already holds a vault key, as an install that signed in before does at the next start.
    fn holding_a_key(key: &[u8; MASTER_KEY_BYTES]) -> Self {
        let store = Self::default();
        *store.wrapped_master_key.lock().unwrap() = Some(key.to_vec());
        store
    }
}

impl AuthSecretStore for MemoryStore {
    fn save_session_token(&self, token: &SessionToken) -> AuthResult<()> {
        *self.session_token.lock().unwrap() = Some(token.clone());
        Ok(())
    }

    fn load_session_token(&self) -> AuthResult<Option<SessionToken>> {
        Ok(self.session_token.lock().unwrap().clone())
    }

    fn delete_session_token(&self) -> AuthResult<()> {
        *self.session_token.lock().unwrap() = None;
        Ok(())
    }

    fn save_wrapped_master_key(&self, wrapped: SecretBytes) -> AuthResult<()> {
        *self.wrapped_master_key.lock().unwrap() = Some(wrapped.expose_for_crypto().to_vec());
        Ok(())
    }

    fn load_wrapped_master_key(&self) -> AuthResult<Option<SecretBytes>> {
        self.wrapped_master_key
            .lock()
            .unwrap()
            .clone()
            .map(SecretBytes::new)
            .transpose()
    }

    fn delete_wrapped_master_key(&self) -> AuthResult<()> {
        *self.wrapped_master_key.lock().unwrap() = None;
        Ok(())
    }
}

/// Port of `debug_output_redacts_session_token_and_key_material`. A session token and a vault key print as a fixed
/// word, in both `Debug` forms, and so does an unlocked vault that holds the key: a log line or a panic message that
/// formats one of them carries no secret. Mutant: derive `Debug` on `SessionToken` (or `SecretBytes`) in place of the
/// redacting impl; the exact-text asserts go red.
#[test]
fn debug_output_redacts_session_token_and_key_material() {
    let token = SessionToken::new("super-secret-session-token").unwrap();
    let secret = SecretBytes::new_master_key(&[9u8; MASTER_KEY_BYTES]);

    assert_eq!(format!("{token:?}"), "SessionToken(<redacted>)");
    assert_eq!(format!("{secret:?}"), "SecretBytes(<redacted>, len=32)");
    assert_eq!(format!("{token:#?}"), "SessionToken(<redacted>)", "the pretty form too");
    assert_eq!(
        format!("{secret:#?}"),
        "SecretBytes(<redacted>, len=32)",
        "the pretty form too"
    );
    assert!(!format!("{token:?}").contains("super-secret"));
    assert!(!format!("{secret:?}").contains('9'));

    // The vault prints its key through the same redaction, also while it is unlocked and holds it in memory.
    let mut vault = AuthVault::new(MemoryStore::holding_a_key(&[9u8; MASTER_KEY_BYTES]));
    vault.unlock().unwrap();
    assert_eq!(vault.master_key().unwrap(), &[9u8; MASTER_KEY_BYTES], "the key is held");
    for shown in [format!("{vault:?}"), format!("{vault:#?}")] {
        assert!(shown.contains("SecretBytes(<redacted>, len=32)"), "{shown}");
        assert!(!shown.contains('9'), "no key byte in {shown}");
    }
}

/// Port of `vault_starts_locked_and_fails_closed`. A new vault is locked and answers no key, and so is one whose store
/// already holds a key: only `unlock()` opens it, and `lock()` closes it again. Mutant: `AuthVault::new` starts
/// `Unlocked`; the first assert goes red.
#[test]
fn vault_starts_locked_and_fails_closed() {
    let vault = AuthVault::new(MemoryStore::default());

    assert_eq!(vault.lock_state(), VaultLockState::Locked);
    assert!(!vault.can_hydrate_or_upload());
    assert!(matches!(
        vault.master_key(),
        Err(AuthStoreError::InvalidSecret("vault is locked"))
    ));

    // A key on file does not open the vault by itself.
    let mut vault = AuthVault::new(MemoryStore::holding_a_key(&[42u8; MASTER_KEY_BYTES]));
    assert_eq!(
        vault.lock_state(),
        VaultLockState::Locked,
        "a stored key is not an open vault"
    );
    assert!(!vault.can_hydrate_or_upload());
    assert!(matches!(
        vault.master_key(),
        Err(AuthStoreError::InvalidSecret("vault is locked"))
    ));

    vault.unlock().unwrap();
    assert_eq!(vault.lock_state(), VaultLockState::Unlocked);
    assert_eq!(vault.master_key().unwrap(), &[42u8; MASTER_KEY_BYTES]);

    vault.lock();
    assert_eq!(vault.lock_state(), VaultLockState::Locked);
    assert!(!vault.can_hydrate_or_upload());
    assert!(matches!(
        vault.master_key(),
        Err(AuthStoreError::InvalidSecret("vault is locked"))
    ));
}
