//! dc-167's red test, at the engine: A signs in, B is added, then A, B, A by the switcher. No switch asks for the
//! twelve words (each account's key comes back from its own R4 slot); signing out of A forgets A's key slot and
//! parked session and keeps B's. Every name is invented.

use std::sync::Arc;

use super::*;
use crate::host::ChannelHost;
use crate::identity::TEST;
use crate::keychain::{SecretBytes, SessionToken};
use crate::signin::memory_store::MemoryCredentials;
use crate::state_paths;

type TestEngine = Engine<MemoryCredentials>;

const A: &str = "0c000000-0000-4000-8000-00000000000a";
const B: &str = "0c000000-0000-4000-8000-00000000000b";
const A_KEY: [u8; 32] = [0xA1; 32];
const B_KEY: [u8; 32] = [0xB2; 32];

struct Pc {
    engine: Arc<TestEngine>,
    _temp: tempfile::TempDir,
}

impl Pc {
    fn new() -> Pc {
        let _ = state_paths::init_for_test();
        let temp = tempfile::tempdir().unwrap();
        let state_dir = temp.path().join("state");
        std::fs::create_dir_all(&state_dir).unwrap();
        let (host, _messages) = ChannelHost::new(|| {});
        let config = super::super::EngineConfig {
            account_id: "dc167-install-slot".into(),
            state_dir: state_dir.clone(),
            api_base: "http://127.0.0.1:9".into(),
        };
        let opened = tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(
            state_paths::STATE_DIR_OVERRIDE.scope(state_dir, async move {
                Engine::open(&TEST, config, host, MemoryCredentials::default())
            }),
        );
        Pc {
            engine: Arc::new(opened.expect("the handle opens")),
            _temp: temp,
        }
    }

    /// A first sign-in of `user_id` on this PC as the sign-in leaves it: token, email, owner, then the key proven
    /// with the twelve words and bound to the account. Then the start's restore.
    fn first_sign_in(&self, user_id: &str, key: &[u8; 32]) {
        let vault = self.engine.vault();
        vault.install_session(SessionToken::new(format!("token-of-{user_id}")).unwrap()).unwrap();
        vault.store_account_email(&format!("{user_id}@example.eu")).unwrap();
        vault.bind_session_owner(user_id).unwrap();
        vault.store_wrapped_master_key(SecretBytes::new_master_key(key)).unwrap();
        vault.bind_vault_key_owner(user_id).unwrap();
        self.engine.restore();
    }

    fn install(&self) -> <MemoryCredentials as CredentialStores>::Store {
        self.engine.stores.for_account(self.engine.identity, &self.engine.config.account_id)
    }

    fn working_key(&self) -> Option<Vec<u8>> {
        self.install().load_wrapped_master_key().unwrap().map(|k| k.expose_for_crypto().to_vec())
    }

    fn kept_key(&self, user_id: &str) -> Option<Vec<u8>> {
        self.install().load_account_key(user_id).unwrap().map(|k| k.expose_for_crypto().to_vec())
    }

    fn token(&self) -> Option<String> {
        self.install().load_session_token().unwrap().map(|t| t.expose_for_request().to_string())
    }

    fn listed(&self) -> Vec<(String, bool)> {
        self.engine.accounts_on_this_pc().into_iter().map(|a| (a.user_id, a.current)).collect()
    }

    /// After a switch to `user_id`: its session and its own key are the install slot's, and nothing asks for the
    /// twelve words.
    fn is_current_without_the_twelve_words(&self, user_id: &str, key: &[u8; 32], gate: GateState) {
        assert_ne!(gate, GateState::NeedsRecoveryPhrase, "a switch to {user_id} asked for the twelve words");
        assert_ne!(gate, GateState::SignedOut);
        assert_eq!(self.token(), Some(format!("token-of-{user_id}")));
        assert_eq!(self.working_key(), Some(key.to_vec()));
        assert_eq!(self.engine.vault().vault_key_owner().unwrap().as_deref(), Some(user_id));
        assert_eq!(
            crate::signin::key_owner::stored_key_state(&self.engine.vault()),
            crate::signin::key_owner::StoredKey::Usable
        );
        assert_eq!(self.engine.account_email(), Some(format!("{user_id}@example.eu")));
    }
}

/// The red test: A, add B, switch to A, switch to B, switch to A. No twelve words, three times.
#[test]
fn account_switch_three_switches_never_ask_for_the_twelve_words() {
    let pc = Pc::new();
    pc.first_sign_in(A, &A_KEY);
    assert_eq!(pc.listed(), vec![(A.to_string(), true)]);

    pc.engine.add_another_account().unwrap();
    assert_eq!(pc.engine.gate(), GateState::SignedOut, "the sign-in shows for the new account");
    assert_eq!(pc.listed(), vec![(A.to_string(), false)], "A still signed in on this PC");
    assert_eq!(pc.kept_key(A), Some(A_KEY.to_vec()), "A's key in A's slot");
    pc.first_sign_in(B, &B_KEY);
    assert_eq!(pc.listed(), vec![(B.to_string(), true), (A.to_string(), false)]);

    for (who, key) in [(A, &A_KEY), (B, &B_KEY), (A, &A_KEY)] {
        let gate = pc.engine.switch_account(who).unwrap();
        pc.is_current_without_the_twelve_words(who, key, gate);
    }
    assert_eq!(pc.listed(), vec![(A.to_string(), true), (B.to_string(), false)]);
    assert_eq!(pc.kept_key(B), Some(B_KEY.to_vec()), "B's key in B's slot");
}

/// Sign out of A while B is current: A's slot and parked session are gone; B's session and key stay.
#[test]
fn account_switch_signing_out_of_a_parked_account_keeps_the_other() {
    let pc = Pc::new();
    pc.first_sign_in(A, &A_KEY);
    pc.engine.add_another_account().unwrap();
    pc.first_sign_in(B, &B_KEY);

    pc.engine.sign_out_parked_account(A).unwrap();

    assert_eq!(pc.kept_key(A), None, "A's key slot is gone");
    assert_eq!(pc.listed(), vec![(B.to_string(), true)]);
    assert_eq!(pc.engine.switch_account(A), Err(SwitchRefused::NotOnThisPc));
    assert_eq!(pc.kept_key(B), Some(B_KEY.to_vec()), "B's slot kept");
    assert_eq!(pc.token(), Some(format!("token-of-{B}")));
    assert_eq!(pc.working_key(), Some(B_KEY.to_vec()));
}

/// Sign out of the current account (the plain sign-out): only it is forgotten; A, parked, still switches back in
/// without the twelve words.
#[test]
fn account_switch_signing_out_of_the_current_account_keeps_the_parked_one() {
    let pc = Pc::new();
    pc.first_sign_in(A, &A_KEY);
    pc.engine.add_another_account().unwrap();
    pc.first_sign_in(B, &B_KEY);

    pc.engine.sign_out_keeping_local_data().unwrap();

    assert_eq!(pc.kept_key(B), None, "B's key slot is gone");
    assert_eq!(pc.kept_key(A), Some(A_KEY.to_vec()), "A's slot kept");
    assert_eq!(pc.listed(), vec![(A.to_string(), false)]);
    let gate = pc.engine.switch_account(A).unwrap();
    pc.is_current_without_the_twelve_words(A, &A_KEY, gate);
}

/// The parked slot's name follows the R4 slot's rule: a server id only, never a path or an email.
#[test]
fn account_switch_parked_slots_are_named_by_server_id_only() {
    assert_eq!(parked_slot_name(A).as_deref(), Some(&*format!("parked-session-{A}")));
    assert_eq!(parked_slot_name("a@example.eu"), None);
    assert_eq!(parked_slot_name("../x"), None);
    assert_eq!(parked_slot_name(""), None);
}
