//! S1, invariant I3 (contract §6): a wrong recovery phrase changes nothing on this PC and sends exactly one request,
//! the server's check. Mutant: let the key through without the proof (`verify_vault_key_from_phrase` ignores a
//! mismatch), so a key is stored for the wrong phrase.

mod common;

#[cfg(not(target_os = "macos"))]
#[test]
fn a_wrong_phrase_writes_nothing() {
    use beebeeb_engine::signin::gate::GateState;
    use zeroize::Zeroizing;

    let s = common::setup();
    s.sign_in();
    let (entries, count) = (s.entries(), s.stores.len());
    assert_eq!(
        entries,
        ["session token", "account email", "session owner"],
        "signed in, no key"
    );
    let from = s.fake.requests().len();

    let refused = s.engine.unlock_with_phrase(Zeroizing::new(common::WRONG_PHRASE.into()));
    assert!(refused.is_err(), "a phrase that is not the account's is refused");

    assert_eq!(s.entries(), entries, "the same entries, by name");
    assert_eq!(s.stores.len(), count, "the same number of entries");
    assert!(!s.state_db().exists(), "no state.db");
    assert_eq!(
        s.routes_from(from),
        ["POST /api/v1/auth/verify-recovery-check"],
        "exactly one request: the check"
    );
    assert_eq!(s.engine.gate(), GateState::NeedsRecoveryPhrase);
    assert_eq!(s.engine.view().gate, GateState::NeedsRecoveryPhrase);
}

#[cfg(target_os = "macos")]
#[test]
fn the_test_build_is_refused_on_macos() {
    common::assert_the_test_build_is_refused();
}
