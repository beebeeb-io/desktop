//! `unlock` tests split out of `keychain.rs`.
//!
//! Startup auto-unlock, account-email and session-clearing tests, split out of
//! `keychain.rs`. Uses the in-memory `MemoryStore` harness that stays in the hub.

use super::*;


    /// Token + 32-byte key present → unlock succeeds and the key is readable.
    /// This is the startup auto-unlock path: relaunch resumes signed-in +
    /// unlocked with no recovery-phrase prompt.
    #[test]
    fn unlock_succeeds_when_master_key_present() {
        let mut vault = AuthVault::new(MemoryStore::default());
        vault.install_session(SessionToken::new("token").unwrap()).unwrap();
        vault
            .store_wrapped_master_key(SecretBytes::new_master_key(&[7u8; MASTER_KEY_BYTES]))
            .unwrap();

        assert_eq!(vault.lock_state(), VaultLockState::Locked);
        vault.unlock().expect("present 32-byte key must unlock");
        assert_eq!(vault.lock_state(), VaultLockState::Unlocked);
        assert_eq!(vault.master_key().unwrap(), &[7u8; MASTER_KEY_BYTES]);
    }

    /// Token present but master key genuinely ABSENT → unlock returns NotFound
    /// and the vault stays locked. This is the only state where onboarding must
    /// still show "this PC doesn't have your keys" / prompt for the recovery
    /// phrase. The startup restore treats this as a no-op.
    #[test]
    fn unlock_reports_not_found_when_master_key_absent() {
        let mut vault = AuthVault::new(MemoryStore::default());
        vault.install_session(SessionToken::new("token").unwrap()).unwrap();
        // No master key stored.

        assert_eq!(vault.unlock(), Err(AuthStoreError::NotFound));
        assert_eq!(vault.lock_state(), VaultLockState::Locked);
        assert!(vault.master_key().is_err());
    }

    /// A stored blob of the wrong length is rejected rather than unlocking with
    /// a malformed key — defensive, in case the credential blob is corrupt.
    #[test]
    fn unlock_rejects_wrong_length_key() {
        let mut vault = AuthVault::new(MemoryStore::default());
        vault.install_session(SessionToken::new("token").unwrap()).unwrap();
        vault
            .store_wrapped_master_key(SecretBytes::new(vec![1u8; 16]).unwrap())
            .unwrap();

        assert!(matches!(vault.unlock(), Err(AuthStoreError::InvalidSecret(_))));
        assert_eq!(vault.lock_state(), VaultLockState::Locked);
    }

    /// The account email round-trips through the store so the auto-unlock path
    /// can recover it: persist on login, read back on relaunch.
    #[test]
    fn account_email_round_trips() {
        let vault = AuthVault::new(MemoryStore::default());
        vault.store_account_email("user@example.com").unwrap();
        assert_eq!(vault.account_email().unwrap().as_deref(), Some("user@example.com"));
    }

    /// Backward-compat: a session stored BEFORE email persistence existed has no
    /// email entry, so `account_email()` returns `None` cleanly — never an error
    /// or panic. This is the existing-stored-session upgrade path.
    #[test]
    fn account_email_absent_returns_none() {
        let vault = AuthVault::new(MemoryStore::default());
        vault.install_session(SessionToken::new("token").unwrap()).unwrap();
        vault
            .store_wrapped_master_key(SecretBytes::new_master_key(&[7u8; MASTER_KEY_BYTES]))
            .unwrap();
        // No email was ever stored.
        assert_eq!(vault.account_email().unwrap(), None);
    }

    /// An empty/whitespace email is treated as "no email" and never written, so
    /// we don't persist a blank credential blob.
    #[test]
    fn empty_account_email_is_not_stored() {
        let vault = AuthVault::new(MemoryStore::default());
        vault.store_account_email("   ").unwrap();
        assert_eq!(vault.account_email().unwrap(), None);
    }

    /// Logout (`clear_session`) wipes the persisted email along with the token
    /// and key — no PII left behind.
    #[test]
    fn clear_session_removes_account_email() {
        let mut vault = AuthVault::new(MemoryStore::default());
        vault.install_session(SessionToken::new("token").unwrap()).unwrap();
        vault
            .store_wrapped_master_key(SecretBytes::new_master_key(&[7u8; MASTER_KEY_BYTES]))
            .unwrap();
        vault.store_account_email("user@example.com").unwrap();
        assert_eq!(vault.account_email().unwrap().as_deref(), Some("user@example.com"));

        vault.clear_session().unwrap();

        assert_eq!(vault.account_email().unwrap(), None);
        assert_eq!(vault.session_token().unwrap(), None);
    }

    /// R8: a token the server rejected at startup is removed alone; the vault key and the email stay.
    #[test]
    fn clearing_the_session_token_keeps_the_key_and_the_email() {
        let mut vault = AuthVault::new(MemoryStore::default());
        vault.install_session(SessionToken::new("tok").unwrap()).unwrap();
        vault
            .store_wrapped_master_key(SecretBytes::new_master_key(&[9u8; 32]))
            .unwrap();
        vault.store_account_email("sam@beebeeb.io").unwrap();
        vault.clear_session_token().unwrap();
        assert!(vault.session_token().unwrap().is_none(), "the revoked token is gone");
        assert_eq!(vault.account_email().unwrap().as_deref(), Some("sam@beebeeb.io"));
        assert!(
            vault.store.load_wrapped_master_key().unwrap().is_some(),
            "R8: the vault key stays"
        );
    }

    /// After the token-only clear of a startup 401, the full clear of "Sign out and switch" leaves no vault key and no
    /// email for the next account.
    #[test]
    fn a_full_clear_after_a_token_only_clear_leaves_no_vault_key() {
        let mut vault = AuthVault::new(MemoryStore::default());
        vault.install_session(SessionToken::new("tok-a").unwrap()).unwrap();
        vault
            .store_wrapped_master_key(SecretBytes::new_master_key(&[7u8; 32]))
            .unwrap();
        vault.store_account_email("sam@beebeeb.io").unwrap();
        vault.clear_session_token().unwrap();
        assert!(
            holds_vault_key(&vault.store),
            "R9: the revoked token goes, the key stays"
        );
        vault.clear_session().unwrap();
        assert!(!holds_vault_key(&vault.store), "after the switch the old key is gone");
        assert_eq!(vault.account_email().unwrap(), None);
    }
