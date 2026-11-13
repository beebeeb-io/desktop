//! `owned_by` (and the handle's refusal while signed out) against a real state database in a folder of the test's own: the owner
//! the engine start records decides, by the binding's own rule; anything that cannot be compared is not the signed-in
//! account's. Every name is invented.

use std::sync::Arc;

use super::super::{Engine, EngineConfig};
use super::owned_by;
use crate::account_binding::Identity;
use crate::host::ChannelHost;
use crate::identity::REAL;
use crate::signin::memory_store::MemoryCredentials;
use crate::state_db::StateDb;

fn db_owned_by(owner: Option<Identity>) -> (tempfile::TempDir, StateDb) {
    let dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    if let Some(owner) = owner {
        db.set_owner(&owner).unwrap();
    }
    (dir, db)
}

fn who(user_id: Option<&str>, email: Option<&str>) -> Identity {
    Identity::new(user_id, email)
}

#[test]
fn the_recorded_owner_is_the_signed_in_account() {
    let (_dir, db) = db_owned_by(Some(who(Some("u-first"), Some("first@example.eu"))));
    assert!(owned_by(&db, &who(Some("u-first"), Some("first@example.eu"))));
    // Before the server named the id: the email decides, by its canonical form.
    assert!(owned_by(&db, &who(None, Some(" First@Example.eu"))));
}

#[test]
fn another_account_never_owns_the_state() {
    let (_dir, db) = db_owned_by(Some(who(Some("u-first"), Some("first@example.eu"))));
    assert!(!owned_by(&db, &who(Some("u-second"), Some("second@example.eu"))));
    assert!(!owned_by(&db, &who(None, Some("second@example.eu"))));
    // The id decides when both sides know it, even with the same email.
    assert!(!owned_by(&db, &who(Some("u-second"), Some("first@example.eu"))));
}

#[test]
fn no_owner_or_no_session_is_not_owned() {
    let (_dir, unowned) = db_owned_by(None);
    assert!(!owned_by(&unowned, &who(Some("u-first"), Some("first@example.eu"))));
    let (_dir, owned) = db_owned_by(Some(who(Some("u-first"), None)));
    assert!(!owned_by(&owned, &Identity::default()), "no one signed in");
    // Nothing to compare: an id on one side, an email on the other.
    assert!(!owned_by(&owned, &who(None, Some("first@example.eu"))));
}

/// The wrapper on a real handle, signed out: no account, so no state is its.
#[test]
fn a_signed_out_handle_owns_no_state() {
    let state_dir = crate::state_paths::init_for_test();
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "an-invented-slot".into(),
        state_dir,
        api_base: "http://127.0.0.1:9".into(),
    };
    let engine = Arc::new(Engine::open(&REAL, config, host, MemoryCredentials::default()).expect("the handle opens"));
    let (dir, db) = db_owned_by(Some(who(Some("u-first"), Some("first@example.eu"))));
    drop(db);
    engine.use_local_state_for_test(dir.path().join("state.db"));
    assert_eq!(
        engine.local_state_refusal(),
        Some(crate::app::LocalStateRefused::SignedOut)
    );
}
