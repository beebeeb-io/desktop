//! The key this computer kept is the stored key, not yet the account's: before a sign-in puts it in memory the
//! server says whether it is still the account's key (one of the account's other devices can have changed it).
//! A key the server confirms is installed; a key it says is outdated is installed nowhere and stays stored, and the
//! twelve words are asked; no answer settles nothing and the sign-in is undone.
//!
//! The rules are core's (`beebeeb_client::auth::kept_key`): whether the kept key may be offered at all
//! ([`may_use_kept_key`]), what the server's answer means ([`verdict`]) and the byte comparison ([`same_key`]). This
//! file gathers the facts and does what core decided.
//!
//! At the start (dc-068, task 2456) a key kept untagged (0.8.x) beside the stored session is proven once against the
//! account the server named for that session, by the same rule: tagged on a real match, left untagged with no check
//! on file, kept stored but closed when the server says it is outdated. A key core says is not this account's is
//! never sent.
//!
//! Must not: log or return a key; load a key before the owner check passed; put a key in memory without a proof.

use std::sync::atomic::Ordering;

use beebeeb_client::auth::kept_key::{
    KeptKeyFacts, KeptKeyUse, KeptKeyVerdict, ProofAnswer, may_use_kept_key, same_key, verdict,
};
use zeroize::Zeroizing;

use super::lock_api::SessionMoved;
use super::{CredentialStores, Engine};
use crate::account::SessionGeneration;
use crate::keychain::{AuthSecretStore, AuthVault};
use crate::signin::gate::{self, NextAfterSignIn};
use crate::signin::{key_owner, recovery, store};

/// What a sign-in found of the key this computer kept, and what the server said about it.
pub(super) enum KeptKey {
    /// No key is stored here: the sign-in asks for the twelve words, as it always did.
    NoneHere,
    /// The server confirmed this key, or the account has no recovery check on file (which proves nothing either way:
    /// the sign-in's rule since the first shell). Only this exact key may be installed. `tag_it` is core's word: only
    /// a key the server really matched takes an owner tag.
    Proven(Zeroizing<[u8; 32]>, bool),
    /// The server says the stored key is no longer the account's. It stays stored; nothing is installed.
    Outdated,
}

/// What the start's proof of an untagged key did ([`Engine::prove_untagged_key_at_start`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StartProof {
    /// Nothing was asked: no stored key, a tagged one, another session's store, or a key core says is not this
    /// account's (never sent under this token).
    NotAsked,
    /// The server matched it: it now carries this account's tag.
    Tagged,
    /// The account has no check on file (or the tag could not be written): it stays untagged and is proven again.
    StaysUntagged,
    /// The server says it is not the account's key: it stays stored, untagged, and the vault is closed when it was the
    /// key in memory, so the next unlock asks for the twelve words.
    Outdated,
    /// No answer: nothing changes.
    Unavailable,
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
        match verdict(proof_answer(answer)) {
            KeptKeyVerdict::Proven { tag_it } => Ok(KeptKey::Proven(key, tag_it)),
            KeptKeyVerdict::Outdated => Ok(KeptKey::Outdated),
            KeptKeyVerdict::Unavailable => Err(KEY_PROOF_UNAVAILABLE.to_string()),
        }
    }

    /// The key in the credential store, only when core's rule says it may be this account's
    /// ([`may_use_kept_key`]: its owner tag is `user_id`, or it has no tag and the email stored beside it is the
    /// signing-in `email`). Anything else, or a tag or email that cannot be read, is NoneHere: it is never asked of
    /// the server under this token, which would let the server link the two accounts. Read BEFORE the sign-in stores
    /// the new email.
    fn read_kept_key(&self, user_id: &str, email: &str) -> Option<Zeroizing<[u8; 32]>> {
        let mut vault = self.vault();
        let owner = vault.vault_key_owner().ok()?;
        let stored_email = vault.account_email().ok()?;
        let facts = KeptKeyFacts {
            owner_tag: owner.as_deref().map(str::as_bytes),
            stored_email: stored_email.as_deref(),
            signing_user_id: user_id,
            signing_email: email,
        };
        if may_use_kept_key(&facts) != KeptKeyUse::ProveIt {
            return None;
        }
        stored_key_bytes(&mut vault)
    }

    /// The stored key of this same account, loaded through the guarded loader once the server confirmed it: the lock
    /// opens in the same hold as the key's install, or stays closed for a session whose key is not here. The key put
    /// in memory is the proven bytes, and only while the stored key is exactly those bytes (task 1866: what the
    /// credential store holds is what the server confirmed).
    pub(super) fn install_kept_key(&self, kept: &KeptKey, email: String) -> NextAfterSignIn {
        let turn: SessionGeneration = self.acct.session_generation();
        let loaded = match kept {
            KeptKey::Proven(proven, _) => store::load_session_from_vault(self.vault(), Some(email))
                .ok()
                .flatten()
                .filter(|session| same_key(&session.master_key, &proven[..]))
                .map(|mut session| {
                    session.master_key = **proven;
                    session
                }),
            KeptKey::NoneHere | KeptKey::Outdated => None,
        };
        if let Ok(_write) = self.claim(turn) {
            if loaded.is_some() && matches!(kept, KeptKey::Proven(_, true)) {
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

    /// The start's proof of a key kept untagged (0.8.x), once the server named the account of the stored session's
    /// `token` (`user_id`, `email`; `turn` is the session asked about). See the module's doc. BLOCKS on the engine's
    /// runtime: call it where `settle_session_owner_now` is called, never from the UI thread.
    pub(in crate::app) fn prove_untagged_key_at_start(
        &self,
        turn: SessionGeneration,
        token: &str,
        user_id: &str,
        email: &str,
    ) -> StartProof {
        let Some(key) = self.untagged_key_to_prove(token, user_id, email) else {
            return StartProof::NotAsked;
        };
        let master = beebeeb_core::kdf::MasterKey::from_bytes(*key);
        let answer = self.runtime.block_on(recovery::recovery_check_answer(
            &self.client,
            &self.config.api_base,
            token,
            &master,
        ));
        self.settle_start_proof(turn, token, user_id, &key, proof_answer(answer))
    }

    /// The stored key to prove at the start: only beside the stored session that is `token`, only an untagged key,
    /// and only when core's rule lets this account offer it.
    fn untagged_key_to_prove(&self, token: &str, user_id: &str, email: &str) -> Option<Zeroizing<[u8; 32]>> {
        let vault = self.vault();
        let stored = vault.session_token().ok().flatten()?;
        if !same_key(stored.expose_for_request().as_bytes(), token.as_bytes()) {
            return None;
        }
        if vault.vault_key_owner().ok()?.is_some() {
            return None;
        }
        self.read_kept_key(user_id, email)
    }

    /// What the start does with the server's `answer` about the untagged key `proven`.
    fn settle_start_proof(
        &self,
        turn: SessionGeneration,
        token: &str,
        user_id: &str,
        proven: &[u8; 32],
        answer: ProofAnswer,
    ) -> StartProof {
        match verdict(answer) {
            KeptKeyVerdict::Unavailable => StartProof::Unavailable,
            KeptKeyVerdict::Proven { tag_it: false } => StartProof::StaysUntagged,
            KeptKeyVerdict::Proven { tag_it: true } => {
                let _writes = self.writes.write().unwrap_or_else(|poisoned| poisoned.into_inner());
                let mut vault = self.vault();
                if !self.acct.session_unchanged_since(turn) || !still_the_untagged_key(&mut vault, token, proven) {
                    return StartProof::NotAsked;
                }
                match vault.bind_vault_key_owner(user_id) {
                    Ok(()) => StartProof::Tagged,
                    Err(error) => {
                        tracing::warn!(%error, "could not record the vault key's owner in the credential store");
                        StartProof::StaysUntagged
                    }
                }
            }
            KeptKeyVerdict::Outdated => {
                let in_memory = self
                    .acct
                    .session
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .as_ref()
                    .is_some_and(|held| held.token == token && same_key(&held.master_key, proven));
                if in_memory && self.acct.session_unchanged_since(turn) {
                    tracing::warn!("the server no longer accepts this computer's kept key; closing the vault");
                    // The stored session and key stay; the next unlock's proof asks for the twelve words.
                    let _ = self.lock_vault();
                }
                StartProof::Outdated
            }
        }
    }
}

/// The 32 key bytes in the credential store, in a buffer wiped on drop.
fn stored_key_bytes<S: AuthSecretStore>(vault: &mut AuthVault<S>) -> Option<Zeroizing<[u8; 32]>> {
    vault.unlock().ok()?;
    let bytes = vault.master_key().ok()?;
    if bytes.len() != 32 {
        return None;
    }
    let mut key = Zeroizing::new([0u8; 32]);
    key.copy_from_slice(bytes);
    Some(key)
}

/// The store still holds `token`, an untagged key, and that key is `proven`: the state the proof was about.
fn still_the_untagged_key<S: AuthSecretStore>(vault: &mut AuthVault<S>, token: &str, proven: &[u8; 32]) -> bool {
    let token_same = vault
        .session_token()
        .ok()
        .flatten()
        .is_some_and(|stored| same_key(stored.expose_for_request().as_bytes(), token.as_bytes()));
    token_same
        && matches!(vault.vault_key_owner(), Ok(None))
        && stored_key_bytes(vault).is_some_and(|key| same_key(&key[..], proven))
}

/// The server's answer, in core's words. Only a completed answer decides.
fn proof_answer(answer: Result<recovery::RecoveryCheckAnswer, String>) -> ProofAnswer {
    use recovery::RecoveryCheckAnswer as Answer;
    match answer {
        Ok(Answer::Matches) => ProofAnswer::Matched,
        Ok(Answer::Mismatch) => ProofAnswer::WrongPhrase,
        Ok(Answer::NoCheckOnFile) => ProofAnswer::NoCheckOnFile,
        Err(_) => ProofAnswer::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::super::{Engine, EngineConfig};
    use crate::host::ChannelHost;
    use crate::identity::TEST;
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
        let engine = Arc::new(Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("opens"));
        let vault = engine.vault();
        vault
            .store_wrapped_master_key(SecretBytes::new_master_key(&[9; 32]))
            .unwrap();
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
        use super::KeptKey;
        use crate::keychain::SessionToken;
        use zeroize::Zeroizing;
        let engine = engine_with_key_and_email(None, Some("me@example.com"));
        let key = engine.read_kept_key("user-a", "me@example.com").expect("read");
        {
            let vault = engine.vault();
            vault
                .install_session(SessionToken::new("tok".to_string()).unwrap())
                .unwrap();
            vault.bind_session_owner("user-a").unwrap();
            assert_eq!(vault.vault_key_owner().unwrap(), None);
        }
        let _ = engine.install_kept_key(&KeptKey::Proven(Zeroizing::new(*key), true), "me@example.com".into());
        assert_eq!(engine.vault().vault_key_owner().unwrap().as_deref(), Some("user-a"));
    }

    #[test]
    fn a_key_with_no_check_on_file_stays_untagged() {
        use super::KeptKey;
        use crate::keychain::SessionToken;
        use zeroize::Zeroizing;
        let engine = engine_with_key_and_email(None, Some("me@example.com"));
        let key = engine.read_kept_key("user-a", "me@example.com").expect("read");
        {
            let vault = engine.vault();
            vault
                .install_session(SessionToken::new("tok".to_string()).unwrap())
                .unwrap();
            vault.bind_session_owner("user-a").unwrap();
        }
        let _ = engine.install_kept_key(&KeptKey::Proven(Zeroizing::new(*key), false), "me@example.com".into());
        assert_eq!(engine.vault().vault_key_owner().unwrap(), None);
    }

    // The start's proof of an untagged key (dc-068, task 2456).

    use super::StartProof;
    use beebeeb_client::auth::kept_key::ProofAnswer;

    /// A stored session "tok" beside the stored key (`owner`, `email`), as a 0.8.x install leaves it.
    fn started_with(owner: Option<&str>, email: Option<&str>) -> Arc<Engine<MemoryCredentials>> {
        use crate::keychain::SessionToken;
        let engine = engine_with_key_and_email(None, email);
        let vault = engine.vault();
        vault
            .install_session(SessionToken::new("tok".to_string()).unwrap())
            .unwrap();
        if let Some(owner) = owner {
            vault.bind_vault_key_owner(owner).unwrap();
        }
        drop(vault);
        engine
    }

    fn settle(engine: &Engine<MemoryCredentials>, answer: ProofAnswer) -> StartProof {
        let key = engine
            .untagged_key_to_prove("tok", "user-a", "me@example.com")
            .expect("an untagged key of this email is proven");
        let turn = engine.acct.session_generation();
        engine.settle_start_proof(turn, "tok", "user-a", &key, answer)
    }

    fn stored_key(engine: &Engine<MemoryCredentials>) -> Option<[u8; 32]> {
        super::stored_key_bytes(&mut engine.vault()).map(|key| *key)
    }

    #[test]
    fn kept_key_at_start_untagged_same_email_is_tagged_on_a_match() {
        let engine = started_with(None, Some(" Me@Example.com"));
        assert_eq!(settle(&engine, ProofAnswer::Matched), StartProof::Tagged);
        assert_eq!(engine.vault().vault_key_owner().unwrap().as_deref(), Some("user-a"));
        assert_eq!(stored_key(&engine), Some([9; 32]));
    }

    #[test]
    fn kept_key_at_start_untagged_other_email_is_never_sent() {
        let engine = started_with(None, Some("other@example.com"));
        assert!(
            engine
                .untagged_key_to_prove("tok", "user-a", "me@example.com")
                .is_none()
        );
        let turn = engine.acct.session_generation();
        assert_eq!(
            engine.prove_untagged_key_at_start(turn, "tok", "user-a", "me@example.com"),
            StartProof::NotAsked
        );
        assert_eq!(engine.vault().vault_key_owner().unwrap(), None);
        let engine = started_with(None, None);
        assert!(
            engine
                .untagged_key_to_prove("tok", "user-a", "me@example.com")
                .is_none()
        );
    }

    #[test]
    fn kept_key_at_start_tagged_keys_are_not_proven_again() {
        let foreign = started_with(Some("user-b"), Some("me@example.com"));
        assert!(
            foreign
                .untagged_key_to_prove("tok", "user-a", "me@example.com")
                .is_none()
        );
        assert_eq!(foreign.vault().vault_key_owner().unwrap().as_deref(), Some("user-b"));
        let own = started_with(Some("user-a"), Some("me@example.com"));
        assert!(own.untagged_key_to_prove("tok", "user-a", "me@example.com").is_none());
    }

    #[test]
    fn kept_key_at_start_only_beside_the_session_asked_about() {
        let engine = started_with(None, Some("me@example.com"));
        assert!(
            engine
                .untagged_key_to_prove("other-tok", "user-a", "me@example.com")
                .is_none()
        );
    }

    #[test]
    fn kept_key_at_start_no_check_on_file_stays_untagged() {
        let engine = started_with(None, Some("me@example.com"));
        assert_eq!(settle(&engine, ProofAnswer::NoCheckOnFile), StartProof::StaysUntagged);
        assert_eq!(engine.vault().vault_key_owner().unwrap(), None);
        assert_eq!(stored_key(&engine), Some([9; 32]));
    }

    #[test]
    fn kept_key_at_start_wrong_phrase_is_outdated_and_the_key_is_kept() {
        let engine = started_with(None, Some("me@example.com"));
        assert_eq!(settle(&engine, ProofAnswer::WrongPhrase), StartProof::Outdated);
        assert_eq!(engine.vault().vault_key_owner().unwrap(), None);
        assert_eq!(stored_key(&engine), Some([9; 32]));
    }

    #[test]
    fn kept_key_at_start_no_answer_changes_nothing() {
        let engine = started_with(None, Some("me@example.com"));
        assert_eq!(settle(&engine, ProofAnswer::Unavailable), StartProof::Unavailable);
        assert_eq!(engine.vault().vault_key_owner().unwrap(), None);
    }

    #[test]
    fn kept_key_at_start_a_key_changed_meanwhile_is_not_tagged() {
        let engine = started_with(None, Some("me@example.com"));
        let turn = engine.acct.session_generation();
        assert_eq!(
            engine.settle_start_proof(turn, "tok", "user-a", &[7; 32], ProofAnswer::Matched),
            StartProof::NotAsked
        );
        assert_eq!(engine.vault().vault_key_owner().unwrap(), None);
    }

    #[test]
    fn kept_key_install_puts_only_the_proven_bytes_in_memory() {
        use super::KeptKey;
        use zeroize::Zeroizing;
        let engine = started_with(None, Some("me@example.com"));
        // A proof about other bytes than the stored key installs nothing (task 1866).
        let _ = engine.install_kept_key(&KeptKey::Proven(Zeroizing::new([7; 32]), true), "me@example.com".into());
        assert!(engine.acct.session.lock().unwrap().is_none());
        assert_eq!(engine.vault().vault_key_owner().unwrap(), None);
        let _ = engine.install_kept_key(
            &KeptKey::Proven(Zeroizing::new([9; 32]), false),
            "me@example.com".into(),
        );
        let held = engine.acct.session.lock().unwrap();
        assert_eq!(held.as_ref().map(|session| session.master_key), Some([9; 32]));
    }
}
