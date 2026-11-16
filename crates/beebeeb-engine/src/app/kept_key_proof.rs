//! The key this computer kept is the stored key, not yet the account's: before a sign-in puts it in memory the
//! server says whether it is still the account's key (one of the account's other devices can have changed it).
//! A key the server confirms is installed; a key it says is outdated is installed nowhere and stays stored, and the
//! twelve words are asked; no answer settles nothing and the sign-in is undone.
//!
//! Must not: log or return a key; load a key before the owner check passed; put a key in memory without a proof.

use std::sync::atomic::Ordering;

use zeroize::Zeroizing;

use super::lock_api::SessionMoved;
use super::{CredentialStores, Engine};
use crate::account::SessionGeneration;
use crate::reauth::KeyProof;
use crate::signin::gate::{self, NextAfterSignIn};
use crate::signin::{key_owner, recovery, store};

/// What a sign-in found of the key this computer kept, and what the server said about it.
pub(super) enum KeptKey {
    /// No key is stored here: the sign-in asks for the twelve words, as it always did.
    NoneHere,
    /// The server confirmed this key (or the account has no recovery check on file, which proves nothing either way:
    /// the sign-in's rule since the first shell). Only this exact key may be installed.
    Proven(Zeroizing<[u8; 32]>, Checked),
    /// The server says the stored key is no longer the account's. It stays stored; nothing is installed.
    Outdated,
}

/// Whether the server really matched a proven key. Only a matched key may be given an owner tag.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Checked {
    Matched,
    NoCheckOnFile,
}

/// Why a sign-in stopped before it stored anything, in the words the window shows.
pub(super) const KEY_PROOF_UNAVAILABLE: &str =
    "Beebeeb could not check this computer\u{2019}s key with the server. Nothing was changed. Try again.";

impl<C: CredentialStores> Engine<C> {
    /// The stored key of the account this sign-in is for, asked of the server under the NEW session's token.
    /// `Err` is the sentence to show: the server gave no answer, so nothing may be decided.
    pub(super) async fn prove_kept_key(&self, token: &str, user_id: &str, email: &str) -> Result<KeptKey, String> {
        let Some(key) = self.read_kept_key(user_id, email) else {
            return Ok(KeptKey::NoneHere);
        };
        let master = beebeeb_core::kdf::MasterKey::from_bytes(*key);
        let answer = recovery::recovery_check_answer(&self.client, &self.config.api_base, token, &master).await;
        match proof_of(answer) {
            KeyProof::Matches => Ok(KeptKey::Proven(key, Checked::Matched)),
            KeyProof::NoCheckOnFile => Ok(KeptKey::Proven(key, Checked::NoCheckOnFile)),
            KeyProof::Mismatch => Ok(KeptKey::Outdated),
            KeyProof::Unavailable => Err(KEY_PROOF_UNAVAILABLE.to_string()),
        }
    }

    /// The key in the credential store, only when it may be this account's: its owner tag is `user_id`, or it has no
    /// tag (kept since before the tags, 0.8.x) and the email stored beside it is the signing-in `email` (the server
    /// keeps one account per canonical address; the same rule as `should_remove_stored_key`). Another account's tag,
    /// an untagged key with another or no email, or a tag that cannot be read is NoneHere: it is never asked of the
    /// server under this token, which would let the server link the two accounts. Read BEFORE the sign-in stores
    /// the new email.
    fn read_kept_key(&self, user_id: &str, email: &str) -> Option<Zeroizing<[u8; 32]>> {
        let mut vault = self.vault();
        match vault.vault_key_owner().ok()?.filter(|owner| !owner.trim().is_empty()) {
            Some(owner) => {
                if !key_owner::tag_is_account(&owner, user_id) {
                    return None;
                }
            }
            None => {
                let stored = vault.account_email().ok().flatten()?;
                if !crate::account_binding::same_email(&stored, email) {
                    return None;
                }
            }
        }
        vault.unlock().ok()?;
        let mut key = Zeroizing::new([0u8; 32]);
        key.copy_from_slice(vault.master_key().ok()?);
        Some(key)
    }

    /// The stored key of this same account, loaded through the guarded loader once the server confirmed it: the lock
    /// opens in the same hold as the key's install, or stays closed for a session whose key is not here. A key that
    /// is not exactly the one the server confirmed is never installed.
    pub(super) fn install_kept_key(&self, kept: &KeptKey, email: String) -> NextAfterSignIn {
        let turn: SessionGeneration = self.acct.session_generation();
        let loaded = match kept {
            KeptKey::Proven(proven, _) => store::load_session_from_vault(self.vault(), Some(email))
                .ok()
                .flatten()
                .filter(|session| session.master_key == **proven),
            KeptKey::NoneHere | KeptKey::Outdated => None,
        };
        if let Ok(_write) = self.claim(turn) {
            if loaded.is_some() && matches!(kept, KeptKey::Proven(_, Checked::Matched)) {
                // A key the server matched that was kept untagged (0.8.x) takes the owner the server named for this
                // session. A key with no check on file proved nothing, so it stays untagged.
                if let Err(error) = key_owner::bind_key_to_session_owner(&self.vault()) {
                    tracing::warn!(%error, "could not record the vault key's owner in the credential store");
                }
            }
            match (loaded, self.acct.session.lock()) {
                (Some(session), Ok(mut guard)) => {
                    *guard = Some(session);
                    drop(guard);
                    self.acct.auth_resume_asked.fetch_add(1, Ordering::SeqCst);
                    self.lock_follows(SessionMoved::StrongProof);
                }
                _ => self.lock_follows(SessionMoved::Keyless),
            }
        }
        gate::next_after_sign_in(false, self.unlocked(), false, false)
    }
}

/// The server's answer, as the sign-in rules read it. Only a completed answer decides.
fn proof_of(answer: Result<recovery::RecoveryCheckAnswer, String>) -> KeyProof {
    use recovery::RecoveryCheckAnswer as Answer;
    match answer {
        Ok(Answer::Matches) => KeyProof::Matches,
        Ok(Answer::Mismatch) => KeyProof::Mismatch,
        Ok(Answer::NoCheckOnFile) => KeyProof::NoCheckOnFile,
        Err(_) => KeyProof::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::super::{Engine, EngineConfig};
    use crate::host::ChannelHost;
    use crate::identity::REAL;
    use crate::keychain::SecretBytes;
    use crate::signin::memory_store::MemoryCredentials;

    fn engine_with_key(owner: Option<&str>) -> Arc<Engine<MemoryCredentials>> {
        engine_with_key_and_email(owner, None)
    }

    fn engine_with_key_and_email(owner: Option<&str>, email: Option<&str>) -> Arc<Engine<MemoryCredentials>> {
        let (host, _messages) = ChannelHost::new(|| {});
        let config = EngineConfig {
            account_id: "a-kept-key-slot".into(),
            state_dir: crate::state_paths::init_for_test(),
            api_base: "http://127.0.0.1:9".into(),
        };
        let engine = Arc::new(Engine::open(&REAL, config, host, MemoryCredentials::default()).expect("opens"));
        let vault = engine.vault();
        vault.store_wrapped_master_key(SecretBytes::new_master_key(&[9; 32])).unwrap();
        if let Some(owner) = owner {
            vault.bind_vault_key_owner(owner).unwrap();
        }
        if let Some(email) = email {
            vault.store_account_email(email).unwrap();
        }
        engine
    }

    #[test]
    fn a_key_of_this_account_is_read() {
        let engine = engine_with_key(Some("user-a"));
        assert!(engine.read_kept_key("user-a", "me@example.com").is_some());
    }

    #[test]
    fn an_untagged_key_is_none_here() {
        let engine = engine_with_key(None);
        assert!(engine.read_kept_key("user-a", "me@example.com").is_none());
    }

    #[test]
    fn another_accounts_key_is_none_here() {
        let engine = engine_with_key(Some("user-b"));
        assert!(engine.read_kept_key("user-a", "me@example.com").is_none());
    }

    #[test]
    fn a_blank_signing_in_id_matches_nothing() {
        let engine = engine_with_key(Some(" "));
        assert!(engine.read_kept_key(" ", "me@example.com").is_none());
    }

    #[test]
    fn an_untagged_key_with_the_same_email_is_read() {
        let engine = engine_with_key_and_email(None, Some("me@example.com"));
        assert!(engine.read_kept_key("user-a", "me@example.com").is_some());
        let engine = engine_with_key_and_email(None, Some("  Me@Example.COM "));
        assert!(engine.read_kept_key("user-a", "me@example.com").is_some());
    }

    #[test]
    fn an_untagged_key_with_another_email_is_none_here() {
        let engine = engine_with_key_and_email(None, Some("other@example.com"));
        assert!(engine.read_kept_key("user-a", "me@example.com").is_none());
    }

    #[test]
    fn an_untagged_key_with_no_email_is_none_here() {
        let engine = engine_with_key_and_email(None, None);
        assert!(engine.read_kept_key("user-a", "me@example.com").is_none());
    }

    #[test]
    fn a_foreign_tag_is_none_here_even_with_the_same_email() {
        let engine = engine_with_key_and_email(Some("user-b"), Some("me@example.com"));
        assert!(engine.read_kept_key("user-a", "me@example.com").is_none());
    }

    #[test]
    fn a_proven_same_email_untagged_key_gets_tagged() {
        use super::{Checked, KeptKey};
        use crate::keychain::SessionToken;
        use zeroize::Zeroizing;
        let engine = engine_with_key_and_email(None, Some("me@example.com"));
        let key = engine.read_kept_key("user-a", "me@example.com").expect("read");
        {
            let vault = engine.vault();
            vault.install_session(SessionToken::new("tok".to_string()).unwrap()).unwrap();
            vault.bind_session_owner("user-a").unwrap();
            assert_eq!(vault.vault_key_owner().unwrap(), None);
        }
        let _ = engine.install_kept_key(&KeptKey::Proven(Zeroizing::new(*key), Checked::Matched), "me@example.com".into());
        assert_eq!(engine.vault().vault_key_owner().unwrap().as_deref(), Some("user-a"));
    }

    #[test]
    fn a_key_with_no_check_on_file_stays_untagged() {
        use super::{Checked, KeptKey};
        use crate::keychain::SessionToken;
        use zeroize::Zeroizing;
        let engine = engine_with_key_and_email(None, Some("me@example.com"));
        let key = engine.read_kept_key("user-a", "me@example.com").expect("read");
        {
            let vault = engine.vault();
            vault.install_session(SessionToken::new("tok".to_string()).unwrap()).unwrap();
            vault.bind_session_owner("user-a").unwrap();
        }
        let _ = engine.install_kept_key(&KeptKey::Proven(Zeroizing::new(*key), Checked::NoCheckOnFile), "me@example.com".into());
        assert_eq!(engine.vault().vault_key_owner().unwrap(), None);
    }
}
