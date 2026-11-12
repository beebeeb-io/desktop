//! `keychain` unit tests, split out of `keychain.rs`.
//!
//! Startup auto-unlock, account email, clearing, lazy legacy migration, presence
//! probes, zeroization and the source-text tests. The in-memory stores live with the
//! area that uses them, so no shared harness crosses module lines.

#[cfg(test)]
mod tests {
    //! Decision-boundary tests for the startup auto-unlock.
    //!
    //! On launch, `lib::restore_session_on_startup` resumes a fully UNLOCKED
    //! session iff the credential store holds both the session token and a
    //! valid 32-byte master key; otherwise it leaves onboarding to prompt for
    //! the recovery phrase. The exact gate is `AuthVault::unlock()`, exercised
    //! here against an in-memory store so it runs on any platform (the real
    //! OS credential vaults are platform-gated and can't run in CI).

    use crate::keychain::*;
    use std::sync::Mutex;

    /// Every platform store names its items under its identity's service, and under no other: the real app's
    /// names are today's bytes (an install in the field finds its key by them), the test build's share no prefix
    /// an OS lookup would match.
    #[test]
    fn a_store_names_its_items_under_its_identitys_service() {
        use crate::identity::{REAL, TEST};
        let real = platform_keychain_store_for(&REAL, "acct-1");
        assert_eq!(
            real.item_name_of(KeychainAccount::SessionToken),
            "io.beebeeb.app/acct-1/session-token"
        );
        assert_eq!(
            real.item_name_of(KeychainAccount::WrappedMasterKey),
            "io.beebeeb.app/acct-1/wrapped-master-key"
        );
        assert_eq!(
            real.item_name_of(KeychainAccount::VaultKeyOwner),
            "io.beebeeb.app/acct-1/vault-key-owner"
        );
        assert_eq!(
            real.item_name_of(KeychainAccount::SessionOwner),
            "io.beebeeb.app/acct-1/session-owner"
        );
        assert_eq!(
            real.item_name_of(KeychainAccount::AccountEmail),
            "io.beebeeb.app/acct-1/account-email"
        );
        let legacy = legacy_platform_keychain_store(&REAL);
        assert_eq!(
            legacy.item_name_of(KeychainAccount::SessionToken),
            "io.beebeeb.app/session-token"
        );
        assert_eq!(
            legacy.item_name_of(KeychainAccount::WrappedMasterKey),
            "io.beebeeb.app/wrapped-master-key"
        );

        let test = platform_keychain_store_for(&TEST, "acct-1");
        let test_legacy = legacy_platform_keychain_store(&TEST);
        for account in KeychainAccount::ALL {
            let (mine, theirs) = (test.item_name_of(account), real.item_name_of(account));
            assert_eq!(mine, format!("io.beebeeb.app.test/acct-1/{}", account.leaf()));
            assert_ne!(mine, theirs);
            assert_eq!(
                test_legacy.item_name_of(account),
                format!("io.beebeeb.app.test/{}", account.leaf())
            );
            assert_ne!(test_legacy.item_name_of(account), legacy.item_name_of(account));
        }
    }

    /// In-memory `AuthSecretStore` standing in for the OS credential vault.
    #[derive(Default)]
    struct MemoryStore {
        token: Mutex<Option<Vec<u8>>>,
        key: Mutex<Option<Vec<u8>>>,
        email: Mutex<Option<String>>,
    }

    impl AuthSecretStore for MemoryStore {
        fn save_session_token(&self, token: &SessionToken) -> AuthResult<()> {
            *self.token.lock().unwrap() = Some(token.expose_for_request().as_bytes().to_vec());
            Ok(())
        }
        fn load_session_token(&self) -> AuthResult<Option<SessionToken>> {
            self.token
                .lock()
                .unwrap()
                .clone()
                .map(|bytes| SessionToken::new(String::from_utf8(bytes).unwrap()))
                .transpose()
        }
        fn delete_session_token(&self) -> AuthResult<()> {
            *self.token.lock().unwrap() = None;
            Ok(())
        }
        fn save_wrapped_master_key(&self, wrapped: SecretBytes) -> AuthResult<()> {
            *self.key.lock().unwrap() = Some(wrapped.expose_for_crypto().to_vec());
            Ok(())
        }
        fn load_wrapped_master_key(&self) -> AuthResult<Option<SecretBytes>> {
            self.key.lock().unwrap().clone().map(SecretBytes::new).transpose()
        }
        fn delete_wrapped_master_key(&self) -> AuthResult<()> {
            *self.key.lock().unwrap() = None;
            Ok(())
        }
        fn save_account_email(&self, email: &str) -> AuthResult<()> {
            *self.email.lock().unwrap() = Some(email.to_string());
            Ok(())
        }
        fn load_account_email(&self) -> AuthResult<Option<String>> {
            Ok(self.email.lock().unwrap().clone())
        }
        fn delete_account_email(&self) -> AuthResult<()> {
            *self.email.lock().unwrap() = None;
            Ok(())
        }
    }

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

    // ── Lazy migration tests (task 0800) ────────────────────────────────────
    //
    // A single shared backing map keyed by the FULL account string models one OS
    // keychain namespace: the legacy store reads/writes bare leaves
    // (`session-token`), the segmented store reads/writes `<id>/session-token`.
    // Two `KeyedMemoryStore` handles over the same `Arc<Mutex<HashMap>>` give us
    // exactly the legacy-vs-segmented separation the real credential vault has.

    use std::collections::HashMap;
    use std::sync::Arc;

    type SharedVault = Arc<Mutex<HashMap<String, Vec<u8>>>>;

    /// In-memory store backed by a shared map, mirroring the per-OS store's
    /// `account_path` prefixing so legacy and segmented entries land at distinct
    /// keys in the same namespace.
    struct KeyedMemoryStore {
        vault: SharedVault,
        prefix: Option<AccountId>,
    }

    impl KeyedMemoryStore {
        fn legacy(vault: SharedVault) -> Self {
            Self { vault, prefix: None }
        }
        fn with_account(vault: SharedVault, id: &str) -> Self {
            Self {
                vault,
                prefix: Some(AccountId::from(id)),
            }
        }
        fn key(&self, account: KeychainAccount) -> String {
            account_path(self.prefix.as_ref(), account)
        }
    }

    impl AuthSecretStore for KeyedMemoryStore {
        fn save_session_token(&self, token: &SessionToken) -> AuthResult<()> {
            self.vault.lock().unwrap().insert(
                self.key(KeychainAccount::SessionToken),
                token.expose_for_request().as_bytes().to_vec(),
            );
            Ok(())
        }
        fn load_session_token(&self) -> AuthResult<Option<SessionToken>> {
            self.vault
                .lock()
                .unwrap()
                .get(&self.key(KeychainAccount::SessionToken))
                .cloned()
                .map(|bytes| SessionToken::new(String::from_utf8(bytes).unwrap()))
                .transpose()
        }
        fn delete_session_token(&self) -> AuthResult<()> {
            self.vault
                .lock()
                .unwrap()
                .remove(&self.key(KeychainAccount::SessionToken));
            Ok(())
        }
        fn save_wrapped_master_key(&self, wrapped: SecretBytes) -> AuthResult<()> {
            self.vault.lock().unwrap().insert(
                self.key(KeychainAccount::WrappedMasterKey),
                wrapped.expose_for_crypto().to_vec(),
            );
            Ok(())
        }
        fn load_wrapped_master_key(&self) -> AuthResult<Option<SecretBytes>> {
            self.vault
                .lock()
                .unwrap()
                .get(&self.key(KeychainAccount::WrappedMasterKey))
                .cloned()
                .map(SecretBytes::new)
                .transpose()
        }
        fn delete_wrapped_master_key(&self) -> AuthResult<()> {
            self.vault
                .lock()
                .unwrap()
                .remove(&self.key(KeychainAccount::WrappedMasterKey));
            Ok(())
        }
        fn save_account_email(&self, email: &str) -> AuthResult<()> {
            self.vault
                .lock()
                .unwrap()
                .insert(self.key(KeychainAccount::AccountEmail), email.as_bytes().to_vec());
            Ok(())
        }
        fn load_account_email(&self) -> AuthResult<Option<String>> {
            Ok(self
                .vault
                .lock()
                .unwrap()
                .get(&self.key(KeychainAccount::AccountEmail))
                .map(|bytes| String::from_utf8(bytes.clone()).unwrap()))
        }
        fn delete_account_email(&self) -> AuthResult<()> {
            self.vault
                .lock()
                .unwrap()
                .remove(&self.key(KeychainAccount::AccountEmail));
            Ok(())
        }
    }

    /// A store that, on demand, drops EVERY write to the wrapped master key —
    /// used to simulate "verify readback fails" without touching a real vault.
    /// Token + email writes still succeed (so verify fails on the KEY specifically).
    struct KeyDropStore {
        inner: KeyedMemoryStore,
    }
    impl AuthSecretStore for KeyDropStore {
        fn save_session_token(&self, token: &SessionToken) -> AuthResult<()> {
            self.inner.save_session_token(token)
        }
        fn load_session_token(&self) -> AuthResult<Option<SessionToken>> {
            self.inner.load_session_token()
        }
        fn delete_session_token(&self) -> AuthResult<()> {
            self.inner.delete_session_token()
        }
        fn save_wrapped_master_key(&self, _wrapped: SecretBytes) -> AuthResult<()> {
            // Silently drop — write "succeeds" but the value never lands, so the
            // verify-readback in step 5 sees a missing key and must NOT delete legacy.
            Ok(())
        }
        fn load_wrapped_master_key(&self) -> AuthResult<Option<SecretBytes>> {
            self.inner.load_wrapped_master_key()
        }
        fn delete_wrapped_master_key(&self) -> AuthResult<()> {
            self.inner.delete_wrapped_master_key()
        }
        fn save_account_email(&self, email: &str) -> AuthResult<()> {
            self.inner.save_account_email(email)
        }
        fn load_account_email(&self) -> AuthResult<Option<String>> {
            self.inner.load_account_email()
        }
        fn delete_account_email(&self) -> AuthResult<()> {
            self.inner.delete_account_email()
        }
    }

    const TEST_ID: &str = "acct-uuid-0001";

    fn seed_legacy(vault: &SharedVault, token: &str, key: Option<[u8; 32]>, email: Option<&str>) {
        let legacy = KeyedMemoryStore::legacy(vault.clone());
        legacy.save_session_token(&SessionToken::new(token).unwrap()).unwrap();
        if let Some(k) = key {
            legacy.save_wrapped_master_key(SecretBytes::new_master_key(&k)).unwrap();
        }
        if let Some(e) = email {
            legacy.save_account_email(e).unwrap();
        }
    }

    fn legacy_present(vault: &SharedVault) -> bool {
        let m = vault.lock().unwrap();
        KeychainAccount::ALL
            .into_iter()
            .any(|account| m.contains_key(account.leaf()))
    }

    /// Happy path: a full legacy trio migrates into the segmented layout, the
    /// segmented entries are present + correct, and the legacy trio is GONE.
    #[test]
    fn migration_happy_path_moves_trio_and_deletes_legacy() {
        let vault: SharedVault = Arc::new(Mutex::new(HashMap::new()));
        seed_legacy(&vault, "tok", Some([9u8; 32]), Some("u@example.com"));

        let new = KeyedMemoryStore::with_account(vault.clone(), TEST_ID);
        let legacy = KeyedMemoryStore::legacy(vault.clone());
        migrate_legacy_keychain_between(&new, &legacy).expect("happy migration");

        // Segmented copies present + correct.
        assert_eq!(new.load_session_token().unwrap().unwrap().expose_for_request(), "tok");
        assert_eq!(
            new.load_wrapped_master_key().unwrap().unwrap().expose_for_crypto(),
            &[9u8; 32]
        );
        assert_eq!(new.load_account_email().unwrap().as_deref(), Some("u@example.com"));
        // Legacy trio gone.
        assert!(
            !legacy_present(&vault),
            "legacy entries must be deleted after migration"
        );
    }

    /// Idempotent: re-running after a complete migration is a no-op that leaves
    /// the segmented trio intact and legacy still gone.
    #[test]
    fn migration_is_idempotent() {
        let vault: SharedVault = Arc::new(Mutex::new(HashMap::new()));
        seed_legacy(&vault, "tok", Some([9u8; 32]), Some("u@example.com"));
        let new = KeyedMemoryStore::with_account(vault.clone(), TEST_ID);
        let legacy = KeyedMemoryStore::legacy(vault.clone());

        migrate_legacy_keychain_between(&new, &legacy).expect("first run");
        // Second run: short-circuits (new token + new key present) → no-op.
        migrate_legacy_keychain_between(&new, &legacy).expect("idempotent re-run");

        assert_eq!(new.load_session_token().unwrap().unwrap().expose_for_request(), "tok");
        assert_eq!(
            new.load_wrapped_master_key().unwrap().unwrap().expose_for_crypto(),
            &[9u8; 32]
        );
        assert!(!legacy_present(&vault));
    }

    /// Interrupted at step 4 (after the new token+key+email were written, before
    /// the legacy delete): the short-circuit sees new token + new key present and
    /// sweeps the leftover legacy entries. Converges to fully-migrated-legacy-gone.
    #[test]
    fn migration_interrupted_before_delete_converges() {
        let vault: SharedVault = Arc::new(Mutex::new(HashMap::new()));
        seed_legacy(&vault, "tok", Some([9u8; 32]), Some("u@example.com"));
        // Simulate a crash right after step 4: new trio written, legacy NOT yet
        // deleted. Pre-write the segmented copies manually to model that state.
        {
            let new = KeyedMemoryStore::with_account(vault.clone(), TEST_ID);
            new.save_session_token(&SessionToken::new("tok").unwrap()).unwrap();
            new.save_wrapped_master_key(SecretBytes::new_master_key(&[9u8; 32]))
                .unwrap();
            new.save_account_email("u@example.com").unwrap();
        }
        assert!(legacy_present(&vault), "precondition: legacy still present pre-rerun");

        let new = KeyedMemoryStore::with_account(vault.clone(), TEST_ID);
        let legacy = KeyedMemoryStore::legacy(vault.clone());
        migrate_legacy_keychain_between(&new, &legacy).expect("re-run converges");

        assert!(!legacy_present(&vault), "leftover legacy swept on re-run");
        assert_eq!(new.load_session_token().unwrap().unwrap().expose_for_request(), "tok");
    }

    /// Interrupted at step 6 (partway through deleting the legacy trio): some
    /// legacy entries remain alongside the full new trio. The short-circuit sweep
    /// finishes the deletion — convergent, idempotent.
    #[test]
    fn migration_interrupted_during_legacy_delete_converges() {
        let vault: SharedVault = Arc::new(Mutex::new(HashMap::new()));
        // New trio complete; legacy token already deleted but legacy key+email
        // still linger (crash mid-sweep).
        {
            let new = KeyedMemoryStore::with_account(vault.clone(), TEST_ID);
            new.save_session_token(&SessionToken::new("tok").unwrap()).unwrap();
            new.save_wrapped_master_key(SecretBytes::new_master_key(&[9u8; 32]))
                .unwrap();
            new.save_account_email("u@example.com").unwrap();
        }
        {
            let legacy = KeyedMemoryStore::legacy(vault.clone());
            // Only key + email remain (token was deleted before the crash).
            legacy
                .save_wrapped_master_key(SecretBytes::new_master_key(&[9u8; 32]))
                .unwrap();
            legacy.save_account_email("u@example.com").unwrap();
        }

        let new = KeyedMemoryStore::with_account(vault.clone(), TEST_ID);
        let legacy = KeyedMemoryStore::legacy(vault.clone());
        migrate_legacy_keychain_between(&new, &legacy).expect("re-run converges");

        assert!(!legacy_present(&vault), "leftover legacy key/email swept");
    }

    /// PARTIAL-WRITE HOLE (the §3 step-2 fix): new token present but new key
    /// MISSING while legacy STILL HAS a key. The short-circuit must NOT declare
    /// victory — it must fall through and re-run 4-6 so the key is recovered.
    #[test]
    fn migration_partial_write_new_token_no_key_falls_through() {
        let vault: SharedVault = Arc::new(Mutex::new(HashMap::new()));
        seed_legacy(&vault, "tok", Some([9u8; 32]), Some("u@example.com"));
        // Model the partial write: new TOKEN landed, new KEY did not.
        {
            let new = KeyedMemoryStore::with_account(vault.clone(), TEST_ID);
            new.save_session_token(&SessionToken::new("tok").unwrap()).unwrap();
            // deliberately NO key write
        }

        let new = KeyedMemoryStore::with_account(vault.clone(), TEST_ID);
        let legacy = KeyedMemoryStore::legacy(vault.clone());
        migrate_legacy_keychain_between(&new, &legacy).expect("falls through and completes");

        // The fall-through must have recovered the key from legacy.
        assert_eq!(
            new.load_wrapped_master_key().unwrap().unwrap().expose_for_crypto(),
            &[9u8; 32],
            "partial-write hole: key must be recovered, not stranded"
        );
        assert!(!legacy_present(&vault), "legacy swept after completing migration");
    }

    /// Verify-fails: the new-key write silently drops, so step 5's readback finds
    /// the key missing. The migration MUST return Err WITHOUT deleting legacy, so
    /// the vault stays usable through the legacy fallback.
    #[test]
    fn migration_verify_fails_keeps_legacy() {
        let vault: SharedVault = Arc::new(Mutex::new(HashMap::new()));
        seed_legacy(&vault, "tok", Some([9u8; 32]), Some("u@example.com"));

        let new = KeyDropStore {
            inner: KeyedMemoryStore::with_account(vault.clone(), TEST_ID),
        };
        let legacy = KeyedMemoryStore::legacy(vault.clone());
        let result = migrate_legacy_keychain_between(&new, &legacy);

        assert!(result.is_err(), "verify-readback failure must surface as Err");
        // CRITICAL: legacy NOT deleted — the vault is still recoverable.
        assert!(
            legacy_present(&vault),
            "legacy MUST survive a failed verify (anti-lockout invariant)"
        );
    }

    /// Fresh/clean install: no legacy token at all → migration is a no-op Ok and
    /// writes nothing. (Models the legacy-fallback's "nothing to migrate" case.)
    #[test]
    fn migration_no_legacy_is_noop() {
        let vault: SharedVault = Arc::new(Mutex::new(HashMap::new()));
        let new = KeyedMemoryStore::with_account(vault.clone(), TEST_ID);
        let legacy = KeyedMemoryStore::legacy(vault.clone());

        migrate_legacy_keychain_between(&new, &legacy).expect("no-op on clean install");

        assert!(new.load_session_token().unwrap().is_none());
        assert!(!legacy_present(&vault));
        assert!(vault.lock().unwrap().is_empty(), "nothing written on a clean install");
    }

    /// LEGACY FALLBACK semantics: after migration the segmented store serves the
    /// session; but a store that only ever had legacy entries (migration skipped)
    /// still reads them through the legacy handle — the read path the anti-lockout
    /// fallback relies on. Token-only legacy (no key) also migrates cleanly.
    #[test]
    fn migration_token_only_legacy_migrates_and_legacy_fallback_reads() {
        let vault: SharedVault = Arc::new(Mutex::new(HashMap::new()));
        // Token-only legacy (an install that signed in but never provisioned a
        // local key — onboarding would prompt for the recovery phrase).
        seed_legacy(&vault, "tok", None, None);

        let new = KeyedMemoryStore::with_account(vault.clone(), TEST_ID);
        let legacy = KeyedMemoryStore::legacy(vault.clone());
        migrate_legacy_keychain_between(&new, &legacy).expect("token-only migration");

        assert_eq!(new.load_session_token().unwrap().unwrap().expose_for_request(), "tok");
        assert!(new.load_wrapped_master_key().unwrap().is_none());
        assert!(!legacy_present(&vault), "legacy token swept after token-only migration");

        // Legacy-fallback read sanity: a legacy handle over a vault that still has
        // a flat token returns it (the fallback the unlock path performs).
        let vault2: SharedVault = Arc::new(Mutex::new(HashMap::new()));
        seed_legacy(&vault2, "legacy-tok", Some([3u8; 32]), Some("e@example.com"));
        let new2 = KeyedMemoryStore::with_account(vault2.clone(), TEST_ID);
        let legacy2 = KeyedMemoryStore::legacy(vault2.clone());
        // Segmented lookup is empty (migration hasn't run) → fall back to legacy.
        assert!(new2.load_session_token().unwrap().is_none());
        assert_eq!(
            legacy2.load_session_token().unwrap().unwrap().expose_for_request(),
            "legacy-tok"
        );
    }

    /// The segmented store keeps an email only next to the token it names. A legacy session migrates with its own
    /// email, or with none when it has none: an email the segmented store already held (next to a token of its own,
    /// or alone) is removed, never kept next to the legacy token.
    #[test]
    fn migration_never_keeps_another_email_next_to_the_legacy_token() {
        for (name, new_token) in [
            ("a segmented token without a key", Some("tok-new")),
            ("an email alone", None),
        ] {
            let vault: SharedVault = Arc::new(Mutex::new(HashMap::new()));
            seed_legacy(&vault, "tok-legacy", Some([9u8; 32]), None);
            {
                let new = KeyedMemoryStore::with_account(vault.clone(), TEST_ID);
                if let Some(token) = new_token {
                    new.save_session_token(&SessionToken::new(token).unwrap()).unwrap();
                }
                new.save_account_email("someone-else@beebeeb.io").unwrap();
            }
            let new = KeyedMemoryStore::with_account(vault.clone(), TEST_ID);
            let legacy = KeyedMemoryStore::legacy(vault.clone());
            migrate_legacy_keychain_between(&new, &legacy).expect("migrates");
            assert_eq!(
                new.load_session_token().unwrap().unwrap().expose_for_request(),
                "tok-legacy",
                "{name}"
            );
            assert_eq!(
                new.load_account_email().unwrap(),
                None,
                "{name}: no email is kept next to the legacy token"
            );
            assert!(!legacy_present(&vault), "{name}");
        }
        // With an email of its own, the legacy session takes it along (the email that was there is replaced).
        let vault: SharedVault = Arc::new(Mutex::new(HashMap::new()));
        seed_legacy(&vault, "tok-legacy", Some([9u8; 32]), Some("u@beebeeb.io"));
        KeyedMemoryStore::with_account(vault.clone(), TEST_ID)
            .save_account_email("someone-else@beebeeb.io")
            .unwrap();
        let new = KeyedMemoryStore::with_account(vault.clone(), TEST_ID);
        migrate_legacy_keychain_between(&new, &KeyedMemoryStore::legacy(vault.clone())).expect("migrates");
        assert_eq!(new.load_account_email().unwrap().as_deref(), Some("u@beebeeb.io"));
    }

    /// A keyless legacy session never lands next to a vault key the segmented store kept (a startup 401 keeps the key
    /// and drops only the token, R9): that key belonged to the segmented session, so it is removed before the legacy
    /// token is written, and the restore can never pair the two (spec §5.6: a key is never left next to another
    /// session). A legacy session with its own key replaces the kept one as before.
    #[test]
    fn a_keyless_legacy_token_never_lands_next_to_a_kept_vault_key() {
        let vault: SharedVault = Arc::new(Mutex::new(HashMap::new()));
        seed_legacy(&vault, "tok-legacy", None, None);
        let new = KeyedMemoryStore::with_account(vault.clone(), TEST_ID);
        new.save_wrapped_master_key(SecretBytes::new_master_key(&[4u8; 32]))
            .unwrap();
        migrate_legacy_keychain_between(&new, &KeyedMemoryStore::legacy(vault.clone())).expect("migrates");
        assert_eq!(
            new.load_session_token().unwrap().unwrap().expose_for_request(),
            "tok-legacy"
        );
        assert!(
            new.load_wrapped_master_key().unwrap().is_none(),
            "the kept key is gone: it is never paired with the legacy token"
        );
        assert!(!legacy_present(&vault));

        let vault: SharedVault = Arc::new(Mutex::new(HashMap::new()));
        seed_legacy(&vault, "tok-legacy", Some([9u8; 32]), None);
        let new = KeyedMemoryStore::with_account(vault.clone(), TEST_ID);
        new.save_wrapped_master_key(SecretBytes::new_master_key(&[4u8; 32]))
            .unwrap();
        migrate_legacy_keychain_between(&new, &KeyedMemoryStore::legacy(vault.clone())).expect("migrates");
        assert_eq!(
            new.load_wrapped_master_key().unwrap().unwrap().expose_for_crypto(),
            &[9u8; 32],
            "a legacy key of its own replaces it"
        );
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
            vault.store().load_wrapped_master_key().unwrap().is_some(),
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
            holds_vault_key(vault.store()),
            "R9: the revoked token goes, the key stays"
        );
        vault.clear_session().unwrap();
        assert!(!holds_vault_key(vault.store()), "after the switch the old key is gone");
        assert_eq!(vault.account_email().unwrap(), None);
    }

    /// Lead ruling 13 (Task 12): the vault key is BORROWED by the constructor that copies it into the wiped buffer, so
    /// no by-value copy of the key is left behind in a frame that nobody wipes.
    #[test]
    fn the_master_key_constructor_borrows_the_key() {
        let source = include_str!("../keychain.rs").replace("\r\n", "\n");
        let at = source
            .find("pub fn new_master_key(bytes: &[u8; MASTER_KEY_BYTES]) -> Self {")
            .expect("the constructor borrows");
        let body = &source[at..];
        let body = &body[body.find('{').unwrap() + 1..body.find("\n    }").unwrap()];
        assert_eq!(
            body.trim(),
            "Self(bytes.to_vec())",
            "the borrowed key goes straight into the wiped buffer, through no local copy"
        );
        // Read without whitespace: a by-value signature with more parameters is wrapped by rustfmt, which puts
        // `bytes:` on a line of its own. (This file is also compiled into `tests/keychain.rs`, where the crate's
        // `source_pin` helper does not exist.)
        let squeezed: String = source.split_whitespace().collect();
        assert!(
            !squeezed.contains(concat!("new_master_key(bytes:[u8", ";")),
            "no by-value constructor"
        );
    }

    /// A retained vault key is seen without unlocking; a store that cannot hold secrets holds none; any other
    /// read error counts as present, so a caller that asks "is anything of an account left?" fails closed.
    #[test]
    fn holds_vault_key_sees_a_retained_key_and_fails_closed() {
        let vault = AuthVault::new(MemoryStore::default());
        assert!(!holds_vault_key(vault.store()));
        vault
            .store_wrapped_master_key(SecretBytes::new_master_key(&[7u8; 32]))
            .unwrap();
        assert!(holds_vault_key(vault.store()));

        struct Answers(fn() -> AuthStoreError);
        impl AuthSecretStore for Answers {
            fn save_session_token(&self, _: &SessionToken) -> AuthResult<()> {
                Err((self.0)())
            }
            fn load_session_token(&self) -> AuthResult<Option<SessionToken>> {
                Err((self.0)())
            }
            fn delete_session_token(&self) -> AuthResult<()> {
                Err((self.0)())
            }
            fn save_wrapped_master_key(&self, _: SecretBytes) -> AuthResult<()> {
                Err((self.0)())
            }
            fn load_wrapped_master_key(&self) -> AuthResult<Option<SecretBytes>> {
                Err((self.0)())
            }
            fn delete_wrapped_master_key(&self) -> AuthResult<()> {
                Err((self.0)())
            }
            fn save_account_email(&self, _: &str) -> AuthResult<()> {
                Err((self.0)())
            }
            fn load_account_email(&self) -> AuthResult<Option<String>> {
                Err((self.0)())
            }
            fn delete_account_email(&self) -> AuthResult<()> {
                Err((self.0)())
            }
        }
        assert!(!holds_vault_key(&Answers(|| AuthStoreError::Unsupported(
            "no keychain here"
        ))));
        assert!(!holds_vault_key(&Answers(|| AuthStoreError::NotFound)));
        assert!(
            holds_vault_key(&Answers(|| AuthStoreError::Backend("keychain locked".into()))),
            "fail closed"
        );
    }

    /// A store that answers the three presence questions and PANICS on any read of the secret itself: a presence check
    /// that reads key or token bytes would fail the test that uses it.
    struct PresenceOnly {
        token: fn() -> AuthResult<bool>,
        key: fn() -> AuthResult<bool>,
        email: fn() -> AuthResult<bool>,
    }

    impl PresenceOnly {
        const ABSENT: fn() -> AuthResult<bool> = || Ok(false);
    }

    impl AuthSecretStore for PresenceOnly {
        fn save_session_token(&self, _: &SessionToken) -> AuthResult<()> {
            unreachable!("no write")
        }
        fn load_session_token(&self) -> AuthResult<Option<SessionToken>> {
            panic!("a presence check must not read the token")
        }
        fn delete_session_token(&self) -> AuthResult<()> {
            unreachable!("no write")
        }
        fn save_wrapped_master_key(&self, _: SecretBytes) -> AuthResult<()> {
            unreachable!("no write")
        }
        fn load_wrapped_master_key(&self) -> AuthResult<Option<SecretBytes>> {
            panic!("a presence check must not read the key")
        }
        fn delete_wrapped_master_key(&self) -> AuthResult<()> {
            unreachable!("no write")
        }
        fn load_account_email(&self) -> AuthResult<Option<String>> {
            panic!("a presence check must not read the email")
        }
        fn holds_session_token(&self) -> AuthResult<bool> {
            (self.token)()
        }
        fn holds_wrapped_master_key(&self) -> AuthResult<bool> {
            (self.key)()
        }
        fn holds_account_email(&self) -> AuthResult<bool> {
            (self.email)()
        }
    }

    /// The three presence probes behind a sign-in's "is anything of an account left?" share one rule: present is
    /// present, a store that cannot hold secrets or has none holds none, and ANY other answer counts as present
    /// (fail closed). Each probe is checked on its own, so a probe that fails open is seen.
    #[test]
    fn every_presence_probe_sees_a_retained_item_and_fails_closed() {
        type Probe = fn(&PresenceOnly) -> bool;
        type Answer = fn() -> AuthResult<bool>;
        type Build = fn(Answer) -> PresenceOnly;
        let probes: [(&str, Probe, Build); 3] = [
            (
                "the session token",
                |s| holds_session_token(s),
                |answer| PresenceOnly {
                    token: answer,
                    key: PresenceOnly::ABSENT,
                    email: PresenceOnly::ABSENT,
                },
            ),
            (
                "the vault key",
                |s| holds_vault_key(s),
                |answer| PresenceOnly {
                    token: PresenceOnly::ABSENT,
                    key: answer,
                    email: PresenceOnly::ABSENT,
                },
            ),
            (
                "the account email",
                |s| holds_account_email(s),
                |answer| PresenceOnly {
                    token: PresenceOnly::ABSENT,
                    key: PresenceOnly::ABSENT,
                    email: answer,
                },
            ),
        ];
        for (name, probe, store_with) in probes {
            assert!(probe(&store_with(|| Ok(true))), "{name}: present is present");
            assert!(!probe(&store_with(|| Ok(false))), "{name}: absent is absent");
            assert!(
                !probe(&store_with(|| Err(AuthStoreError::Unsupported("no keychain here")))),
                "{name}: a store that cannot hold it holds none"
            );
            assert!(
                !probe(&store_with(|| Err(AuthStoreError::NotFound))),
                "{name}: not found is absent"
            );
            assert!(
                probe(&store_with(|| Err(AuthStoreError::Backend("keychain locked".into())))),
                "{name}: a locked store fails closed"
            );
            assert!(
                probe(&store_with(|| Err(AuthStoreError::InvalidSecret("not UTF-8")))),
                "{name}: an unreadable item fails closed"
            );
        }
    }

    /// A store that implements only the reads (every older store) gets presence from the default, which reads the
    /// item and drops it at once: its answers follow the read's, errors included.
    #[test]
    fn the_default_presence_answers_follow_the_reads_and_fail_closed() {
        let vault = AuthVault::new(MemoryStore::default());
        assert!(
            !holds_session_token(vault.store())
                && !holds_vault_key(vault.store())
                && !holds_account_email(vault.store())
        );
        vault.install_session(SessionToken::new("tok").unwrap()).unwrap();
        vault.store_account_email("sam@beebeeb.io").unwrap();
        assert!(
            holds_session_token(vault.store()) && holds_account_email(vault.store()) && !holds_vault_key(vault.store())
        );

        struct Reads(fn() -> AuthStoreError);
        impl AuthSecretStore for Reads {
            fn save_session_token(&self, _: &SessionToken) -> AuthResult<()> {
                Err((self.0)())
            }
            fn load_session_token(&self) -> AuthResult<Option<SessionToken>> {
                Err((self.0)())
            }
            fn delete_session_token(&self) -> AuthResult<()> {
                Err((self.0)())
            }
            fn save_wrapped_master_key(&self, _: SecretBytes) -> AuthResult<()> {
                Err((self.0)())
            }
            fn load_wrapped_master_key(&self) -> AuthResult<Option<SecretBytes>> {
                Err((self.0)())
            }
            fn delete_wrapped_master_key(&self) -> AuthResult<()> {
                Err((self.0)())
            }
            fn save_account_email(&self, _: &str) -> AuthResult<()> {
                Err((self.0)())
            }
            fn load_account_email(&self) -> AuthResult<Option<String>> {
                Err((self.0)())
            }
            fn delete_account_email(&self) -> AuthResult<()> {
                Err((self.0)())
            }
        }
        let locked = Reads(|| AuthStoreError::Backend("keychain locked".into()));
        assert!(
            holds_session_token(&locked) && holds_vault_key(&locked) && holds_account_email(&locked),
            "a read error counts as present"
        );
        let nothing = Reads(|| AuthStoreError::NotFound);
        assert!(!holds_session_token(&nothing) && !holds_vault_key(&nothing) && !holds_account_email(&nothing));
    }

    /// The session token wipes itself: `zeroize` empties its text, and its `Drop` calls it.
    #[test]
    fn a_session_token_is_wiped_when_it_goes() {
        use zeroize::Zeroize as _;
        let mut token = SessionToken::new("a-secret-session-token").unwrap();
        token.zeroize();
        assert_eq!(token.expose_for_request(), "", "the text is gone");
        let source = include_str!("../keychain/session.rs").replace("\r\n", "\n");
        let drop_impl = &source[source
            .find("impl Drop for SessionToken {")
            .expect("the token wipes on drop")..];
        let drop_impl = &drop_impl[..drop_impl.find("\n}\n").unwrap()];
        assert!(drop_impl.contains("zeroize::Zeroize::zeroize(self)"), "{drop_impl}");
    }

    /// A secret is wiped with the `zeroize` crate (volatile writes the optimizer may not drop), never with a plain fill
    /// before the free, which a compiler may remove as a dead store: `SecretBytes` wipes itself on drop, the macOS read
    /// that fails to free, and the Windows read that fails to decode. No plain fill is left in this file.
    #[test]
    fn secrets_are_wiped_with_zeroize_never_a_plain_fill() {
        use zeroize::Zeroize as _;
        let mut key = SecretBytes::new_master_key(&[7u8; MASTER_KEY_BYTES]);
        key.zeroize();
        assert!(key.expose_for_crypto().is_empty(), "the bytes are gone");
        let source = concat!(
            include_str!("../keychain.rs"),
            "\n",
            include_str!("../keychain/session.rs"),
            "\n",
            include_str!("../keychain/vault.rs"),
            "\n",
            include_str!("../keychain/legacy.rs"),
            "\n",
            include_str!("../keychain/accounts.rs"),
            "\n",
            include_str!("../keychain/errors.rs"),
            "\n",
        )
        .replace("\r\n", "\n");
        let plain_fill = concat!(".fill", "(0)");
        assert_eq!(
            source.matches(plain_fill).count(),
            0,
            "a plain fill is still used to wipe a secret"
        );
        let secret_bytes_drop = &source[source
            .find("impl Drop for SecretBytes {")
            .expect("SecretBytes wipes on drop")..];
        let secret_bytes_drop = &secret_bytes_drop[..secret_bytes_drop.find("\n}\n").unwrap()];
        assert!(
            secret_bytes_drop.contains("zeroize::Zeroize::zeroize(self)"),
            "{secret_bytes_drop}"
        );
        let windows = &source[source
            .find("impl AuthSecretStore for WindowsCredentialStore {")
            .expect("the Windows store")..];
        let windows = &windows[..windows.find("\n}\n").unwrap()];
        assert!(
            windows.contains("zeroize::Zeroize::zeroize(&mut raw)"),
            "the Windows read that fails to decode wipes with zeroize:\n{windows}"
        );
    }

    /// The real macOS Keychain answers "is it there?" from the item's attributes (`find_item`), never by reading the
    /// secret, and a free that fails after a read wipes what it copied before it reports the error.
    #[test]
    fn the_real_macos_store_answers_presence_without_reading_the_secret() {
        let source = include_str!("../keychain.rs").replace("\r\n", "\n");
        let store = &source[source
            .find("impl AuthSecretStore for MacOsKeychainStore {")
            .expect("the real store")..];
        let store = &store[..store.find("\n}\n").unwrap()];
        for presence in [
            "fn holds_session_token(",
            "fn holds_wrapped_master_key(",
            "fn holds_account_email(",
        ] {
            let body = &store[store
                .find(presence)
                .unwrap_or_else(|| panic!("the real store answers {presence}"))..];
            let body = &body[..body.find("\n    }").unwrap_or(body.len())];
            assert!(
                body.contains("macos_keychain::exists("),
                "{presence} asks the item's attributes:\n{body}"
            );
            assert!(
                !body.contains("::load("),
                "{presence} must not read the secret:\n{body}"
            );
        }
        // The FFI module is the last thing in the file, after this test, so the LAST match is the real one.
        let macos = &source[source.rfind("mod macos_keychain {").expect("the macOS FFI module")..];
        let ffi = &macos[macos
            .find("pub fn exists(account: &str)")
            .expect("the attribute-only query")..];
        let exists = &ffi[..ffi.find("\n    }\n").unwrap()];
        assert!(
            exists.contains("find_item(") && !exists.contains("password_data"),
            "{exists}"
        );
        let load = &macos[macos.find("pub fn load(account: &str)").unwrap()..];
        let load = &load[..load.find("\n    }\n").unwrap()];
        assert!(
            load.contains("zeroize::Zeroize::zeroize(&mut bytes)"),
            "a failed free wipes the copy before the error is returned:\n{load}"
        );
    }
}
