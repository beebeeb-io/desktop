//! Task 2447: the ownership rules, driven through the real `AuthVault` and the real loader over an in-memory store
//! (one that can refuse a delete or a read, as a credential store that fails does). Runs on every OS: it never
//! touches the platform store. Tokens and keys here are filler bytes, not credentials.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use super::*;
use crate::keychain::{SecretBytes, SessionToken};

const TOKEN: &str = "session-token";
const KEY: &str = "wrapped-master-key";
const EMAIL: &str = "account-email";
const KEY_OWNER: &str = "vault-key-owner";
const SESSION_OWNER: &str = "session-owner";

#[derive(Default)]
struct Inner {
    items: HashMap<&'static str, Vec<u8>>,
    deletes_fail: HashSet<&'static str>,
    reads_fail: HashSet<&'static str>,
    /// When set, the write of this item fails: "the process died before this write" for the crash tests.
    writes_fail: HashSet<&'static str>,
}

#[derive(Clone, Default)]
struct MemStore(Arc<Mutex<Inner>>);

impl MemStore {
    fn save(&self, leaf: &'static str, bytes: &[u8]) -> AuthResult<()> {
        let mut inner = self.0.lock().unwrap();
        if inner.writes_fail.contains(leaf) {
            return Err(AuthStoreError::Backend(format!("write of {leaf} refused")));
        }
        inner.items.insert(leaf, bytes.to_vec());
        Ok(())
    }
    fn load(&self, leaf: &'static str) -> AuthResult<Option<Vec<u8>>> {
        let inner = self.0.lock().unwrap();
        if inner.reads_fail.contains(leaf) {
            return Err(AuthStoreError::Backend(format!("read of {leaf} refused")));
        }
        Ok(inner.items.get(leaf).cloned())
    }
    fn delete(&self, leaf: &'static str) -> AuthResult<()> {
        let mut inner = self.0.lock().unwrap();
        if inner.deletes_fail.contains(leaf) {
            return Err(AuthStoreError::Backend(format!("delete of {leaf} refused")));
        }
        inner.items.remove(leaf);
        Ok(())
    }
    fn text(&self, leaf: &'static str) -> AuthResult<Option<String>> {
        Ok(self.load(leaf)?.map(|bytes| String::from_utf8(bytes).unwrap()))
    }
    fn has(&self, leaf: &'static str) -> bool {
        self.0.lock().unwrap().items.contains_key(leaf)
    }
    fn fail_deletes(&self, leaf: &'static str, fail: bool) {
        let mut inner = self.0.lock().unwrap();
        if fail {
            inner.deletes_fail.insert(leaf)
        } else {
            inner.deletes_fail.remove(leaf)
        };
    }
    fn fail_reads(&self, leaf: &'static str) {
        self.0.lock().unwrap().reads_fail.insert(leaf);
    }
    fn fail_writes(&self, leaf: &'static str) {
        self.0.lock().unwrap().writes_fail.insert(leaf);
    }
    fn vault(&self) -> AuthVault<MemStore> {
        AuthVault::new(self.clone())
    }
}

impl AuthSecretStore for MemStore {
    fn save_session_token(&self, token: &SessionToken) -> AuthResult<()> {
        self.save(TOKEN, token.expose_for_request().as_bytes())
    }
    fn load_session_token(&self) -> AuthResult<Option<SessionToken>> {
        self.text(TOKEN)?.map(SessionToken::new).transpose()
    }
    fn delete_session_token(&self) -> AuthResult<()> {
        self.delete(TOKEN)
    }
    fn save_wrapped_master_key(&self, wrapped: SecretBytes) -> AuthResult<()> {
        self.save(KEY, wrapped.expose_for_crypto())
    }
    fn load_wrapped_master_key(&self) -> AuthResult<Option<SecretBytes>> {
        self.load(KEY)?.map(SecretBytes::new).transpose()
    }
    fn delete_wrapped_master_key(&self) -> AuthResult<()> {
        self.delete(KEY)
    }
    fn save_account_email(&self, email: &str) -> AuthResult<()> {
        self.save(EMAIL, email.as_bytes())
    }
    fn load_account_email(&self) -> AuthResult<Option<String>> {
        self.text(EMAIL)
    }
    fn delete_account_email(&self) -> AuthResult<()> {
        self.delete(EMAIL)
    }
    fn save_vault_key_owner(&self, user_id: &str) -> AuthResult<()> {
        self.save(KEY_OWNER, user_id.as_bytes())
    }
    fn load_vault_key_owner(&self) -> AuthResult<Option<String>> {
        self.text(KEY_OWNER)
    }
    fn delete_vault_key_owner(&self) -> AuthResult<()> {
        self.delete(KEY_OWNER)
    }
    fn save_session_owner(&self, user_id: &str) -> AuthResult<()> {
        self.save(SESSION_OWNER, user_id.as_bytes())
    }
    fn load_session_owner(&self) -> AuthResult<Option<String>> {
        self.text(SESSION_OWNER)
    }
    fn delete_session_owner(&self) -> AuthResult<()> {
        self.delete(SESSION_OWNER)
    }
}

const KEY_A: [u8; 32] = [0xA1; 32];
const KEY_B: [u8; 32] = [0xB2; 32];

fn put_token(store: &MemStore, token: &str) -> AuthResult<()> {
    AuthVault::install_session(&store.vault(), SessionToken::new(token).unwrap())
}
fn put_key(store: &MemStore, key: &[u8; 32]) -> AuthResult<()> {
    AuthVault::store_wrapped_master_key(&store.vault(), SecretBytes::new_master_key(key))
}
/// Account A fully signed in the way `install_new_session` leaves it: token, key, email, both owners.
fn signed_in_as_a() -> MemStore {
    let store = MemStore::default();
    put_token(&store, "tok-a").unwrap();
    put_key(&store, &KEY_A).unwrap();
    store.vault().store_account_email("a@beebeeb.io").unwrap();
    bind_session_and_key(&store.vault(), "user-a").unwrap();
    store
}
/// What a pre-2447 build left: token, key and email, no tags.
fn legacy_a() -> MemStore {
    let store = MemStore::default();
    put_token(&store, "tok-a").unwrap();
    put_key(&store, &KEY_A).unwrap();
    store.vault().store_account_email("a@beebeeb.io").unwrap();
    store
}
/// The token-only sign-in `store_first_sign_in` does: the foreign-key check, then the token, then its owner.
fn token_only_sign_in(store: &MemStore, token: &str, user_id: &str, email: &str) -> AuthResult<bool> {
    let removed = remove_foreign_key(&mut store.vault(), user_id, email)?;
    store.delete(EMAIL)?;
    put_token(store, token)?;
    store.vault().store_account_email(email)?;
    store.vault().bind_session_owner(user_id)?;
    Ok(removed)
}
/// The REAL loader (`load_session_from_vault`, the shared tail of `load_session_from_keychain`).
fn load(store: &MemStore) -> Result<Option<crate::session::Session>, String> {
    crate::signin::store::load_session_from_vault(store.vault(), None)
}

// ── the pure comparison ─────────────────────────────────────────────────────────

#[test]
fn an_untagged_key_is_unknown_and_never_a_match() {
    assert_eq!(key_ownership(Some("user-a"), Some("user-a")), KeyOwnership::Matches);
    assert_eq!(key_ownership(Some("user-a"), Some("user-b")), KeyOwnership::Foreign);
    assert_eq!(key_ownership(Some("user-a"), None), KeyOwnership::SessionUnknown);
    for session in [None, Some("user-a"), Some("")] {
        assert_eq!(key_ownership(None, session), KeyOwnership::Unknown);
        assert_eq!(
            key_ownership(Some("  "), session),
            KeyOwnership::Unknown,
            "a blank tag is no tag"
        );
        assert_ne!(key_ownership(None, session), KeyOwnership::Matches);
    }
    assert!(KeyOwnership::Matches.may_load() && KeyOwnership::Unknown.may_load());
    assert!(!KeyOwnership::Foreign.may_load() && !KeyOwnership::SessionUnknown.may_load());
}

#[test]
fn which_stored_key_a_token_only_sign_in_removes() {
    // Tagged: only its own user id keeps it, whatever the emails say.
    assert!(!should_remove_stored_key(
        Some("user-a"),
        "user-a",
        "new@beebeeb.io",
        Some("a@beebeeb.io")
    ));
    assert!(should_remove_stored_key(
        Some("user-a"),
        "user-b",
        "a@beebeeb.io",
        Some("a@beebeeb.io")
    ));
    // Untagged: only the same canonical email keeps it.
    assert!(!should_remove_stored_key(
        None,
        "user-a",
        "A@Beebeeb.io ",
        Some("a@beebeeb.io")
    ));
    assert!(should_remove_stored_key(
        None,
        "user-b",
        "b@beebeeb.io",
        Some("a@beebeeb.io")
    ));
    assert!(
        should_remove_stored_key(None, "user-b", "b@beebeeb.io", None),
        "nothing says whose it is"
    );
    assert!(
        should_remove_stored_key(None, "user-b", "", Some("")),
        "blank is not the same address"
    );
}

// ── the task's scenarios, through the vault and the loader ──────────────────────────

/// Task 2447's repro: A's clear fails and leaves A's key; B signs in by password.
#[test]
fn a_stale_key_and_a_different_account_the_key_is_removed_and_the_gate_asks_for_the_phrase() {
    let store = signed_in_as_a();
    // The clear that fails: the token goes, the key's removal is refused.
    store.fail_deletes(KEY, true);
    assert!(store.vault().clear_session().is_err());
    store.fail_deletes(KEY, false);
    assert!(store.has(KEY) && !store.has(TOKEN), "A's key outlived A's session");

    assert_eq!(token_only_sign_in(&store, "tok-b", "user-b", "b@beebeeb.io"), Ok(true));
    assert!(!store.has(KEY) && !store.has(KEY_OWNER), "A's key and its tag are gone");
    assert_eq!(stored_key_state(&store.vault()), StoredKey::Absent);
    assert_eq!(
        crate::signin::gate::decide_gate(true, false, stored_key_state(&store.vault()), false),
        crate::signin::gate::GateState::NeedsRecoveryPhrase
    );
    // The loader has no key to give, so no session reaches memory and no engine can start (`keys_for_engine_start`
    // reads only the session in memory).
    assert!(load(&store).is_err());
}

#[test]
fn the_same_account_keeps_its_key_and_loads_it() {
    let store = signed_in_as_a();
    store.vault().clear_session_token().unwrap();
    assert_eq!(
        token_only_sign_in(&store, "tok-a2", "user-a", "a@beebeeb.io"),
        Ok(false)
    );
    assert_eq!(stored_key_ownership(&store.vault()), Ok(KeyOwnership::Matches));
    assert_eq!(stored_key_state(&store.vault()), StoredKey::Usable);
    let session = load(&store).unwrap().expect("a session");
    assert_eq!((session.token.as_str(), session.master_key), ("tok-a2", KEY_A));
}

#[test]
fn a_legacy_untagged_key_and_the_same_email_is_kept_and_tagged_only_after_a_verified_unlock() {
    let store = legacy_a();
    assert_eq!(
        token_only_sign_in(&store, "tok-a2", "user-a", "A@beebeeb.io"),
        Ok(false)
    );
    assert!(
        store.has(KEY) && !store.has(KEY_OWNER),
        "kept, and NOT tagged by the sign-in"
    );
    assert_eq!(stored_key_ownership(&store.vault()), Ok(KeyOwnership::Unknown));
    assert_eq!(load(&store).unwrap().expect("loads as before").master_key, KEY_A);
    assert!(!store.has(KEY_OWNER), "a load proves nothing: still untagged");
    // The startup probe names the session; that is not a proof of the key either.
    assert_eq!(
        settle_owner_after_probe(&store.vault(), "tok-a2", "user-a"),
        Ok(KeyOwnership::Unknown)
    );
    assert!(!store.has(KEY_OWNER));
    // The recovery check proves the phrase's key: it is stored, and only now tagged.
    put_key(&store, &KEY_A).unwrap();
    assert_eq!(bind_key_to_session_owner(&store.vault()), Ok(true));
    assert_eq!(stored_key_ownership(&store.vault()), Ok(KeyOwnership::Matches));
}

#[test]
fn a_legacy_untagged_key_and_a_different_or_missing_email_is_removed() {
    let store = legacy_a();
    assert_eq!(token_only_sign_in(&store, "tok-b", "user-b", "b@beebeeb.io"), Ok(true));
    assert!(!store.has(KEY));

    let no_email = legacy_a();
    no_email.delete(EMAIL).unwrap();
    assert_eq!(
        token_only_sign_in(&no_email, "tok-b", "user-b", "b@beebeeb.io"),
        Ok(true)
    );
    assert!(!no_email.has(KEY));
}

/// The loader's own refusal, with the guard at the sign-in bypassed (a token written by a path that never asked).
#[test]
fn the_loader_refuses_a_key_bound_to_another_user_id() {
    let store = signed_in_as_a();
    put_token(&store, "tok-b").unwrap();
    store.vault().bind_session_owner("user-b").unwrap();
    assert_eq!(stored_key_ownership(&store.vault()), Ok(KeyOwnership::Foreign));
    assert_eq!(load(&store).unwrap_err(), KEY_NOT_THIS_ACCOUNTS);
    assert_eq!(
        stored_key_state(&store.vault()),
        StoredKey::NotThisAccounts,
        "the gate agrees with the loader"
    );

    // A token nobody named, next to a tagged key: refused too.
    let unnamed = signed_in_as_a();
    put_token(&unnamed, "tok-b").unwrap();
    assert_eq!(stored_key_ownership(&unnamed.vault()), Ok(KeyOwnership::SessionUnknown));
    assert_eq!(load(&unnamed).unwrap_err(), KEY_NOT_THIS_ACCOUNTS);

    // A store that cannot say whose key it is: refused, and its own gate state.
    let unreadable = signed_in_as_a();
    unreadable.fail_reads(KEY_OWNER);
    assert!(load(&unreadable).is_err());
    assert_eq!(stored_key_state(&unreadable.vault()), StoredKey::Unreadable);
    assert_eq!(
        crate::signin::gate::decide_gate(true, false, StoredKey::Unreadable, false),
        crate::signin::gate::GateState::KeyStoreUnavailable
    );
}

#[test]
fn a_failed_removal_refuses_the_sign_in_and_stores_nothing() {
    let store = signed_in_as_a();
    store.vault().clear_session_token().unwrap();
    store.fail_deletes(KEY, true);
    assert!(token_only_sign_in(&store, "tok-b", "user-b", "b@beebeeb.io").is_err());
    assert!(!store.has(TOKEN), "B's token was not stored next to A's key");
    assert!(store.has(KEY));
    // Once the store answers again, the same sign-in removes the key and goes through.
    store.fail_deletes(KEY, false);
    assert_eq!(token_only_sign_in(&store, "tok-b", "user-b", "b@beebeeb.io"), Ok(true));
}

// ── the write order: no crash state pairs a key or a token with another one's tag ─────────

#[test]
fn a_new_key_never_carries_the_old_keys_tag() {
    // Crash after the key, before its tag: unknown, never "user-a".
    let store = signed_in_as_a();
    put_key(&store, &KEY_B).unwrap();
    assert_eq!(store.vault().vault_key_owner(), Ok(None));
    assert_eq!(stored_key_ownership(&store.vault()), Ok(KeyOwnership::Unknown));

    // The old tag cannot be removed: the new key is NOT written.
    let stuck = signed_in_as_a();
    stuck.fail_deletes(KEY_OWNER, true);
    assert!(put_key(&stuck, &KEY_B).is_err());
    assert_eq!(
        stuck.load(KEY).unwrap().unwrap(),
        KEY_A.to_vec(),
        "still A's key under A's tag"
    );

    // Crash after the tag's removal, before the key's write: the old key, untagged.
    let died = signed_in_as_a();
    died.fail_writes(KEY);
    assert!(put_key(&died, &KEY_B).is_err());
    assert_eq!(died.load(KEY).unwrap().unwrap(), KEY_A.to_vec());
    assert_eq!(died.vault().vault_key_owner(), Ok(None));
}

#[test]
fn a_new_token_never_carries_the_old_sessions_tag() {
    let store = signed_in_as_a();
    put_token(&store, "tok-b").unwrap();
    assert_eq!(
        store.vault().session_owner(),
        Ok(None),
        "crash before the new owner is bound: unknown"
    );

    let stuck = signed_in_as_a();
    stuck.fail_deletes(SESSION_OWNER, true);
    assert!(put_token(&stuck, "tok-b").is_err());
    assert_eq!(
        stuck.text(TOKEN).unwrap().as_deref(),
        Some("tok-a"),
        "the old token stays under its own tag"
    );

    // A crash between the two binds of a token+key install: the session is named, the key is unknown.
    let half = MemStore::default();
    put_token(&half, "tok-b").unwrap();
    put_key(&half, &KEY_B).unwrap();
    half.fail_writes(KEY_OWNER);
    assert!(bind_session_and_key(&half.vault(), "user-b").is_err());
    assert_eq!(stored_key_ownership(&half.vault()), Ok(KeyOwnership::Unknown));
}

#[test]
fn removing_a_key_never_leaves_another_key_under_its_tag() {
    let store = signed_in_as_a();
    store.fail_deletes(KEY_OWNER, true);
    assert!(store.vault().delete_vault_key_and_owner().is_err());
    assert!(!store.has(KEY) && store.has(KEY_OWNER), "a tag with no key");
    // The leftover tag stops the next key until it can be removed, and then goes first.
    assert!(put_key(&store, &KEY_B).is_err());
    store.fail_deletes(KEY_OWNER, false);
    put_key(&store, &KEY_B).unwrap();
    assert_eq!(store.vault().vault_key_owner(), Ok(None));
}

#[test]
fn a_clear_tries_every_item_even_when_one_refuses() {
    let store = signed_in_as_a();
    store.fail_deletes(TOKEN, true);
    assert!(store.vault().clear_session().is_err());
    assert!(store.has(TOKEN));
    for leaf in [KEY, EMAIL, KEY_OWNER, SESSION_OWNER] {
        assert!(!store.has(leaf), "{leaf} did not stay behind with the token");
    }
}

// ── the startup probe, the pending clear, rotation, P2-2 ───────────────────────────

#[test]
fn the_probe_names_the_session_and_judges_the_key_by_the_servers_id() {
    let legacy = legacy_a();
    assert_eq!(
        settle_owner_after_probe(&legacy.vault(), "tok-a", "user-a"),
        Ok(KeyOwnership::Unknown)
    );
    assert_eq!(legacy.vault().session_owner().unwrap().as_deref(), Some("user-a"));

    let foreign = signed_in_as_a();
    assert_eq!(
        settle_owner_after_probe(&foreign.vault(), "tok-a", "user-b"),
        Ok(KeyOwnership::Foreign)
    );
    assert_eq!(
        foreign.vault().session_owner().unwrap().as_deref(),
        Some("user-b"),
        "the server's word replaces the recorded owner"
    );
    assert_eq!(
        load(&foreign).unwrap_err(),
        KEY_NOT_THIS_ACCOUNTS,
        "and the loader refuses from now on"
    );

    // Another token was stored while the probe waited: nothing is written.
    let changed = signed_in_as_a();
    assert_eq!(
        settle_owner_after_probe(&changed.vault(), "tok-old", "user-b"),
        Ok(KeyOwnership::SessionUnknown)
    );
    assert_eq!(changed.vault().session_owner().unwrap().as_deref(), Some("user-a"));
}

#[test]
fn a_clear_that_failed_is_finished_at_the_next_start() {
    let store = signed_in_as_a();
    store.fail_deletes(KEY, true);
    assert!(store.vault().clear_session().is_err());
    // Next start: the store still refuses, so it stays pending (and a sign-in is refused, see above).
    assert!(complete_pending_clear(&mut store.vault()).is_err());
    store.fail_deletes(KEY, false);
    assert_eq!(complete_pending_clear(&mut store.vault()), Ok(true));
    assert!(!store.has(KEY) && !store.has(KEY_OWNER) && !store.has(SESSION_OWNER));
    assert_eq!(
        complete_pending_clear(&mut store.vault()),
        Ok(false),
        "nothing left to finish"
    );
    // A stored session is never touched, and a token that cannot be read is not "no token".
    let live = signed_in_as_a();
    assert_eq!(complete_pending_clear(&mut live.vault()), Ok(false));
    assert!(live.has(KEY));
    live.fail_reads(TOKEN);
    assert!(complete_pending_clear(&mut live.vault()).is_err());
    assert!(live.has(KEY));
}

#[test]
fn a_rotated_token_keeps_its_sessions_owner_only_for_the_token_it_replaces() {
    let store = signed_in_as_a();
    assert_eq!(session_owner_of(&store.vault(), "tok-other"), None);
    let owner = session_owner_of(&store.vault(), "tok-a").expect("the rotating session's owner");
    put_token(&store, "tok-a-rotated").unwrap();
    assert_eq!(stored_key_ownership(&store.vault()), Ok(KeyOwnership::SessionUnknown));
    store.vault().bind_session_owner(&owner).unwrap();
    assert_eq!(
        load(&store).unwrap().expect("unlocks after a restart").master_key,
        KEY_A
    );
}

#[test]
fn an_approval_adds_the_key_only_to_the_same_signed_in_account_while_it_is_locked() {
    assert!(may_add_key_to_signed_in_session(Some("user-a"), Some("user-a"), false));
    assert!(
        !may_add_key_to_signed_in_session(Some("user-a"), Some("user-b"), false),
        "another account"
    );
    assert!(
        !may_add_key_to_signed_in_session(Some("user-a"), Some("user-a"), true),
        "already unlocked"
    );
    assert!(
        !may_add_key_to_signed_in_session(None, Some("user-a"), false),
        "nobody named the signed-in session"
    );
    assert!(
        !may_add_key_to_signed_in_session(Some("user-a"), None, false),
        "no profile for the approval"
    );
    assert!(!may_add_key_to_signed_in_session(Some(""), Some(""), false));

    // The install that follows (`install_new_session`): token and key, both bound; the loader accepts it.
    let store = MemStore::default();
    token_only_sign_in(&store, "tok-a", "user-a", "a@beebeeb.io").unwrap();
    assert_eq!(stored_key_state(&store.vault()), StoredKey::Absent);
    put_token(&store, "tok-a-approved").unwrap();
    put_key(&store, &KEY_A).unwrap();
    bind_session_and_key(&store.vault(), "user-a").unwrap();
    assert_eq!(stored_key_ownership(&store.vault()), Ok(KeyOwnership::Matches));
    assert_eq!(load(&store).unwrap().expect("a session").master_key, KEY_A);
}

/// Review (delta 3): a store that cannot be READ must not block every sign-in while a delete would work.
#[test]
fn an_unreadable_leftover_is_removed_not_trusted_and_not_a_permanent_refusal() {
    for unreadable in [KEY, KEY_OWNER, EMAIL] {
        let store = signed_in_as_a();
        store.vault().clear_session_token().unwrap();
        store.fail_reads(unreadable);
        assert!(
            token_only_sign_in(&store, "tok-a2", "user-a", "a@beebeeb.io").is_ok(),
            "{unreadable}"
        );
        assert!(
            !store.has(KEY) && !store.has(KEY_OWNER),
            "{unreadable}: not shown to be this account's, so removed"
        );
    }
    let pending = signed_in_as_a();
    pending.vault().clear_session_token().unwrap();
    pending.fail_reads(KEY);
    assert_eq!(complete_pending_clear(&mut pending.vault()), Ok(true));
    assert!(!pending.has(KEY));
}

/// Review (delta 3): a tag write that fails after the probe does not turn the person's own key into a foreign one.
#[test]
fn a_session_tag_that_cannot_be_written_after_the_probe_does_not_refuse_the_key() {
    let legacy = legacy_a();
    legacy.fail_writes(SESSION_OWNER);
    assert_eq!(
        settle_owner_after_probe(&legacy.vault(), "tok-a", "user-a"),
        Ok(KeyOwnership::Unknown)
    );
    let tagged = signed_in_as_a();
    tagged.delete(SESSION_OWNER).unwrap();
    tagged.fail_writes(SESSION_OWNER);
    assert_eq!(
        settle_owner_after_probe(&tagged.vault(), "tok-a", "user-a"),
        Ok(KeyOwnership::Matches)
    );
}
