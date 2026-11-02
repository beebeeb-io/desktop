//! `presence` tests split out of `keychain.rs`.
//!
//! Presence-probe tests and the `PresenceOnly` store that answers presence
//! directly, split out of `keychain.rs`.

use super::*;


    /// A retained vault key is seen without unlocking; a store that cannot hold secrets holds none; any other
    /// read error counts as present, so a caller that asks "is anything of an account left?" fails closed.
    #[test]
    fn holds_vault_key_sees_a_retained_key_and_fails_closed() {
        let vault = AuthVault::new(MemoryStore::default());
        assert!(!holds_vault_key(&vault.store));
        vault
            .store_wrapped_master_key(SecretBytes::new_master_key(&[7u8; 32]))
            .unwrap();
        assert!(holds_vault_key(&vault.store));

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
            !holds_session_token(&vault.store) && !holds_vault_key(&vault.store) && !holds_account_email(&vault.store)
        );
        vault.install_session(SessionToken::new("tok").unwrap()).unwrap();
        vault.store_account_email("sam@beebeeb.io").unwrap();
        assert!(
            holds_session_token(&vault.store) && holds_account_email(&vault.store) && !holds_vault_key(&vault.store)
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
