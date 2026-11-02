//! `migration` tests split out of `keychain.rs`.
//!
//! Lazy legacy-keychain migration tests and their keyed in-memory stores,
//! split out of `keychain.rs`.

use super::*;


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
        prefix: Option<String>,
    }

    impl KeyedMemoryStore {
        fn legacy(vault: SharedVault) -> Self {
            Self { vault, prefix: None }
        }
        fn with_account(vault: SharedVault, id: &str) -> Self {
            Self {
                vault,
                prefix: Some(id.to_string()),
            }
        }
        fn key(&self, leaf: &str) -> String {
            account_path(self.prefix.as_deref(), leaf)
        }
    }

    impl AuthSecretStore for KeyedMemoryStore {
        fn save_session_token(&self, token: &SessionToken) -> AuthResult<()> {
            self.vault.lock().unwrap().insert(
                self.key(SESSION_TOKEN_ACCOUNT),
                token.expose_for_request().as_bytes().to_vec(),
            );
            Ok(())
        }
        fn load_session_token(&self) -> AuthResult<Option<SessionToken>> {
            self.vault
                .lock()
                .unwrap()
                .get(&self.key(SESSION_TOKEN_ACCOUNT))
                .cloned()
                .map(|bytes| SessionToken::new(String::from_utf8(bytes).unwrap()))
                .transpose()
        }
        fn delete_session_token(&self) -> AuthResult<()> {
            self.vault.lock().unwrap().remove(&self.key(SESSION_TOKEN_ACCOUNT));
            Ok(())
        }
        fn save_wrapped_master_key(&self, wrapped: SecretBytes) -> AuthResult<()> {
            self.vault.lock().unwrap().insert(
                self.key(WRAPPED_MASTER_KEY_ACCOUNT),
                wrapped.expose_for_crypto().to_vec(),
            );
            Ok(())
        }
        fn load_wrapped_master_key(&self) -> AuthResult<Option<SecretBytes>> {
            self.vault
                .lock()
                .unwrap()
                .get(&self.key(WRAPPED_MASTER_KEY_ACCOUNT))
                .cloned()
                .map(SecretBytes::new)
                .transpose()
        }
        fn delete_wrapped_master_key(&self) -> AuthResult<()> {
            self.vault.lock().unwrap().remove(&self.key(WRAPPED_MASTER_KEY_ACCOUNT));
            Ok(())
        }
        fn save_account_email(&self, email: &str) -> AuthResult<()> {
            self.vault
                .lock()
                .unwrap()
                .insert(self.key(ACCOUNT_EMAIL_ACCOUNT), email.as_bytes().to_vec());
            Ok(())
        }
        fn load_account_email(&self) -> AuthResult<Option<String>> {
            Ok(self
                .vault
                .lock()
                .unwrap()
                .get(&self.key(ACCOUNT_EMAIL_ACCOUNT))
                .map(|bytes| String::from_utf8(bytes.clone()).unwrap()))
        }
        fn delete_account_email(&self) -> AuthResult<()> {
            self.vault.lock().unwrap().remove(&self.key(ACCOUNT_EMAIL_ACCOUNT));
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
        m.contains_key(SESSION_TOKEN_ACCOUNT)
            || m.contains_key(WRAPPED_MASTER_KEY_ACCOUNT)
            || m.contains_key(ACCOUNT_EMAIL_ACCOUNT)
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
